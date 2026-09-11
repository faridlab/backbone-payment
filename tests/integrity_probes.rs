//! Integrity probes for payment — invariants that must hold against a REAL Postgres beyond the
//! golden math. Requires DATABASE_URL (:5433/backbone_payment).
//!
//! IP-1..IP-3   the posting/settlement invariants (service level).
//! IGT-1..IGT-3 the tenancy invariants on the guarded HTTP surface: a payment's tenant is derived
//!              from a signed token, never from the request body.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use backbone_auth::org::OrgVerifier;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use std::future::Future;

/// Drive a settlement/reversal verb inside the org request scope the composition layer always
/// binds (ADR-0029) — those verbs read the legacy company twin off the ambient scope.
async fn in_org_scope<R>(pool: &PgPool, f: impl Future<Output = R>) -> R {
    backbone_orm::org_scope::with_org_request_scope(
        pool,
        backbone_orm::org_scope::OrgScope::for_company_unit(Uuid::new_v4()),
        f,
    )
    .await
    .expect("bind org request scope")
}

use backbone_payment::presentation::http::create_guarded_payment_routes;
use backbone_payment::PaymentModule;

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostRejected, GlPostSink,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};

/// Tests inject the reconcilability read — the real probe reads accounting, which these
/// fixtures do not populate; landing-state behavior is asserted via the stub and in the
/// lifecycle suite.
struct AlwaysReconcilable;
#[async_trait::async_trait]
impl backbone_payment::application::service::payment_lifecycle::BankReconcilablePort
    for AlwaysReconcilable
{
    async fn bank_reconcilable(
        &self,
        _pool: &sqlx::PgPool,
        _company_id: uuid::Uuid,
        _account_id: uuid::Uuid,
    ) -> Result<bool, backbone_payment::application::service::payment_write_service::PaymentError>
    {
        Ok(true)
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}
fn day() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
}
fn uq(p: &str) -> String {
    format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..8])
}
async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost:5433/backbone_payment".to_string()
    });
    PgPool::connect(&url).await.expect("connect DB")
}

struct RejectingGl;
#[async_trait::async_trait]
impl GlPostSink for RejectingGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Err(GlPostRejected {
            code: "period_closed".into(),
            message: "accounting period is closed".into(),
        })
    }
}
#[derive(Clone)]
struct OkGl {
    journal: Uuid,
    post: Uuid,
}
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Ok(GlPostAck {
            post_id: self.post,
            journal_id: self.journal,
            idempotent_reuse: false,
        })
    }
}
/// Blocks on a barrier BEFORE returning the ack — makes the pending→posted UPDATE race deterministic.
#[derive(Clone)]
struct BarrierGl {
    gate: Arc<tokio::sync::Barrier>,
    journal: Uuid,
    post: Uuid,
}
#[async_trait::async_trait]
impl GlPostSink for BarrierGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        self.gate.wait().await;
        Ok(GlPostAck {
            post_id: self.post,
            journal_id: self.journal,
            idempotent_reuse: false,
        })
    }
}
#[derive(Default, Clone)]
struct Recorder {
    events: Arc<Mutex<Vec<PaymentEvent>>>,
}
impl PaymentEventSink for Recorder {
    fn publish(&self, e: PaymentEvent) {
        self.events.lock().unwrap().push(e);
    }
}

fn receive(currency: Option<String>) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"),
        branch_id: None,
        payment_type: "receive".into(),
        party_type: Some("customer".into()),
        party_id: Some(Uuid::new_v4()),
        posting_date: day(),
        currency,
        mode_of_payment_id: None,
        method: None,
        provider_txn_id: None,
        bank_account_id: Uuid::new_v4(),
        party_account_id: Uuid::new_v4(),
        paid_amount: d("100000"),
        reference_no: None,
        allocations: vec![NewAllocation {
            invoice_ref: Uuid::new_v4(),
            invoice_kind: "sales".into(),
            amount: d("100000"),
        }],
        withholding_amount: rust_decimal::Decimal::ZERO,
        withholding_account_id: None,
        withholding_tax_type: "none".into(),
    }
}

// IP-1: a rejected GL post leaves the payment NOT posted and recoverable — posting_state=failed,
// status still draft, no journal. A later good sink completes it (failed is retryable), landing the
// fused status: reconcilable bank + manual method ⇒ in_flight.
#[tokio::test]
async fn rejected_post_is_recoverable() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone())
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let id = w
        .create_payment(receive(None))
        .await
        .unwrap();
    let e = in_org_scope(&pool, w.post_payment(id, &RejectingGl)).await.unwrap_err();
    assert!(matches!(e, PaymentError::GlRejected { .. }));
    let (ps, st, jid): (String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT posting_state::text, status::text, journal_id FROM payment.payment_entries WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(ps, "failed");
    assert_eq!(st, "draft");
    assert!(jid.is_none());

    in_org_scope(&pool, w.post_payment(
        id,
        &OkGl {
            journal: Uuid::new_v4(),
            post: Uuid::new_v4(),
        },
    ))
    .await
    .unwrap();
    let (ps2, st2): (String, String) = sqlx::query_as(
        "SELECT posting_state::text, status::text FROM payment.payment_entries WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ps2, "posted");
    assert_eq!(st2, "in_flight");
}

// IP-2: a non-IDR payment is refused at post time; no mis-valued post reaches the ledger.
#[tokio::test]
async fn non_idr_refused_at_post() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone())
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let id = w
        .create_payment(receive(Some("USD".into())))
        .await
        .unwrap();
    let e = in_org_scope(&pool, w
        .post_payment(
            id,
            &OkGl {
                journal: Uuid::new_v4(),
                post: Uuid::new_v4(),
            },
        ))
        .await
        .unwrap_err();
    assert!(matches!(e, PaymentError::UnsupportedCurrency(c) if c == "USD"));
}

// IP-3: the seam event is emitted EXACTLY once under a concurrent double-post — the pending→posted
// gate stops a double `PaymentSettled` that would draw an invoice's outstanding down twice via
// billing::apply_settlement. (Proactively applied from billing's council finding.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_post_emits_settled_once() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = Arc::new(
        PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone()))
            .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable)),
    );
    let id = w
        .create_payment(receive(None))
        .await
        .unwrap();
    let gl = BarrierGl {
        gate: Arc::new(tokio::sync::Barrier::new(2)),
        journal: Uuid::new_v4(),
        post: Uuid::new_v4(),
    };
    let (w1, w2, g1, g2, p1, p2) =
        (w.clone(), w.clone(), gl.clone(), gl.clone(), pool.clone(), pool.clone());
    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { in_org_scope(&p1, w1.post_payment(id, &g1)).await }),
        tokio::spawn(async move { in_org_scope(&p2, w2.post_payment(id, &g2)).await }),
    );
    r1.unwrap().unwrap();
    r2.unwrap().unwrap();
    let emitted = rec
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PaymentEvent::PaymentSettled(s) if s.payment_id == id))
        .count();
    assert_eq!(
        emitted, 1,
        "the settlement event must fire exactly once, even under a concurrent double-post"
    );
}

// IP-4: a posted payment can be reversed — reverse_payment posts the sign-flipped mirror journal,
// transitions posted→cancelled, and emits PaymentCancelled. A second reverse is a no-op.
#[tokio::test]
async fn posted_payment_is_reversible_and_idempotent() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone()))
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let id = w.create_payment(receive(None)).await.unwrap();
    in_org_scope(&pool, w.post_payment(
        id,
        &OkGl {
            journal: Uuid::new_v4(),
            post: Uuid::new_v4(),
        },
    ))
    .await
    .unwrap();

    // Reverse it.
    let outcome = in_org_scope(&pool, w
        .reverse_payment(
            id,
            &OkGl {
                journal: Uuid::new_v4(),
                post: Uuid::new_v4(),
            },
        ))
        .await
        .unwrap();
    assert!(
        !outcome.idempotent_reuse,
        "first reverse must succeed (not an idempotent reuse)"
    );

    // DB state: status=cancelled, posting_state still posted (the reversal post succeeded).
    let (status, posting_state): (String, String) = sqlx::query_as(
        "SELECT status::text, posting_state::text FROM payment.payment_entries WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "cancelled");
    assert_eq!(posting_state, "posted");

    // PaymentCancelled emitted.
    let cancelled = rec
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PaymentEvent::PaymentCancelled(c) if c.payment_id == id))
        .count();
    assert_eq!(cancelled, 1, "PaymentCancelled must fire exactly once");

    // A second reverse is a no-op (already cancelled).
    let outcome2 = in_org_scope(&pool, w
        .reverse_payment(
            id,
            &OkGl {
                journal: Uuid::new_v4(),
                post: Uuid::new_v4(),
            },
        ))
        .await
        .unwrap();
    assert!(
        outcome2.idempotent_reuse,
        "second reverse must be an idempotent no-op"
    );
}

// ── guarded HTTP surface: tenancy ────────────────────────────────────────────

const SECRET: &[u8] = b"payment-integrity-probe-secret";

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_unit_id: Option<Uuid>,
}

/// Mint an HS256 org access token. `org_unit_id = None` models a token that authenticates a user
/// but names no acting unit — it must not be allowed to move money.
fn token(org_unit_id: Option<Uuid>) -> String {
    let claims = TestClaims {
        sub: "probe-user".into(),
        exp: 9_999_999_999,
        org_unit_id,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(SECRET),
    )
    .unwrap()
}

async fn module(pool: &PgPool) -> PaymentModule {
    PaymentModule::builder()
        .with_database(pool.clone())
        .build()
        .unwrap()
}
fn app(pool: &PgPool, m: &PaymentModule) -> axum::Router {
    create_guarded_payment_routes(m, pool.clone(), OrgVerifier::hs256(SECRET))
}

/// Send a request with an optional bearer token. The request carries the tenant-database extension
/// `org_auth` reads — the composing service's tenant router provides it in a real deployment.
async fn req_with(
    app: axum::Router,
    pool: &PgPool,
    method: &str,
    uri: &str,
    body: Option<String>,
    bearer: Option<String>,
) -> (StatusCode, String) {
    let b = body.map(Body::from).unwrap_or(Body::empty());
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let mut req = builder.body(b).unwrap();
    req.extensions_mut().insert(pool.clone());
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// A well-formed receive-payment body. `company_id` is deliberately absent — the tenant rides on the
/// token. `smuggled_company` injects a `companyId` an attacker would hope the handler trusts.
fn payment_body(number: &str, smuggled_company: Option<Uuid>) -> String {
    let smuggled = smuggled_company
        .map(|c| format!(r#""companyId":"{c}","branchId":"{}","#, Uuid::new_v4()))
        .unwrap_or_default();
    format!(
        r#"{{"paymentNumber":"{}",{}"paymentType":"receive","partyType":"customer","partyId":"{}",
             "postingDate":"2026-07-05","bankAccountId":"{}","partyAccountId":"{}","paidAmount":"100000",
             "allocations":[{{"invoiceRef":"{}","invoiceKind":"sales","amount":"100000"}}]}}"#,
        number,
        smuggled,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    )
}

// IGT-1: an unauthenticated write is rejected. Before the tenant guard this create succeeded and
// stamped whatever `companyId` the caller put in the body.
#[tokio::test]
async fn guarded_write_rejects_unauthenticated() {
    let pool = pool().await;
    let m = module(&pool).await;
    let (status, _) = req_with(
        app(&pool, &m),
        &pool,
        "POST",
        "/payment-entries",
        Some(payment_body(&uq("PE"), None)),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an unauthenticated write must not reach the service"
    );
}

// IGT-2: a token that authenticates a user but carries no `org_unit_id` claim is rejected — a
// writer with no resolvable session must never run.
#[tokio::test]
async fn guarded_write_rejects_token_without_org_unit() {
    let pool = pool().await;
    let m = module(&pool).await;
    let (status, _) = req_with(
        app(&pool, &m),
        &pool,
        "POST",
        "/payment-entries",
        Some(payment_body(&uq("PE"), None)),
        Some(token(None)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a token with no acting unit must not write"
    );
}

// IGT-3: a `companyId` smuggled in the body cannot name a tenant — there is nothing left for it to
// override. The module carries no tenancy of its own (ADR-0029): the write body has no tenant
// field, the tables carry no tenant column, and the only tenancy that ever applies is the session
// `org_auth` resolves — refused here fail-closed, since this probe's database holds no organization
// tree for the token's unit.
#[tokio::test]
async fn body_company_id_cannot_name_the_tenant() {
    let pool = pool().await;
    let m = module(&pool).await;
    let attacker_company = Uuid::new_v4();
    let number = uq("PE");

    // Same well-formed body, plus a `companyId` no code path reads any more — and a token naming a
    // unit this database has no tree for: the guard refuses the session before any handler runs.
    let (status, body) = req_with(
        app(&pool, &m),
        &pool,
        "POST",
        "/payment-entries",
        Some(payment_body(&number, Some(attacker_company))),
        Some(token(Some(Uuid::new_v4()))),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::CREATED,
        "a session the guard could not resolve must never write, got: {body}"
    );

    // And structurally: the payment tables carry no tenant column the body could have stamped.
    let hits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns \
         WHERE table_schema='payment' AND table_name='payment_entries' AND column_name='company_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("column probe");
    assert_eq!(
        hits, 0,
        "payment.payment_entries must carry no tenant column (ADR-0029)"
    );
}
