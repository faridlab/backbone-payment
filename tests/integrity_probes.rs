//! Integrity probes for payment — invariants that must hold against a REAL Postgres beyond the
//! golden math. Requires DATABASE_URL (:5433/backbone_payment).
//!
//! IP-1..IP-3   the posting/settlement invariants (service level).
//! IGT-1..IGT-3 the tenancy invariants on the guarded HTTP surface: a payment's tenant is derived
//!              from a signed token, never from the request body.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use backbone_auth::company::CompanyVerifier;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use backbone_payment::presentation::http::create_guarded_payment_routes;
use backbone_payment::PaymentModule;

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostRejected, GlPostSink,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};

fn d(s: &str) -> Decimal { Decimal::from_str_exact(s).unwrap() }
fn day() -> chrono::NaiveDate { chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap() }
fn uq(p: &str) -> String { format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..8]) }
async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5433/backbone_payment".to_string());
    PgPool::connect(&url).await.expect("connect DB")
}

struct RejectingGl;
#[async_trait::async_trait]
impl GlPostSink for RejectingGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Err(GlPostRejected { code: "period_closed".into(), message: "accounting period is closed".into() })
    }
}
#[derive(Clone)]
struct OkGl { journal: Uuid, post: Uuid }
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Ok(GlPostAck { post_id: self.post, journal_id: self.journal, idempotent_reuse: false })
    }
}
/// Blocks on a barrier BEFORE returning the ack — makes the pending→posted UPDATE race deterministic.
#[derive(Clone)]
struct BarrierGl { gate: Arc<tokio::sync::Barrier>, journal: Uuid, post: Uuid }
#[async_trait::async_trait]
impl GlPostSink for BarrierGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        self.gate.wait().await;
        Ok(GlPostAck { post_id: self.post, journal_id: self.journal, idempotent_reuse: false })
    }
}
#[derive(Default, Clone)]
struct Recorder { events: Arc<Mutex<Vec<PaymentEvent>>> }
impl PaymentEventSink for Recorder {
    fn publish(&self, e: PaymentEvent) { self.events.lock().unwrap().push(e); }
}

fn receive(company: Uuid, currency: Option<String>) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
        party_type: Some("customer".into()), party_id: Some(Uuid::new_v4()), posting_date: day(), currency,
        mode_of_payment_id: None, bank_account_id: Uuid::new_v4(), party_account_id: Uuid::new_v4(),
        paid_amount: d("100000"), reference_no: None,
        allocations: vec![NewAllocation { invoice_ref: Uuid::new_v4(), invoice_kind: "sales".into(), amount: d("100000") }],
    }
}

// IP-1: a rejected GL post leaves the payment NOT posted and recoverable — posting_state=failed,
// status still draft, no journal. A later good sink completes it.
#[tokio::test]
async fn rejected_post_is_recoverable() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone());
    let id = w.create_payment(receive(Uuid::new_v4(), None)).await.unwrap();
    let e = w.post_payment(id, &RejectingGl).await.unwrap_err();
    assert!(matches!(e, PaymentError::GlRejected { .. }));
    let (ps, st, jid): (String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT posting_state::text, status::text, journal_id FROM payment.payment_entries WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(ps, "failed");
    assert_eq!(st, "draft");
    assert!(jid.is_none());

    w.post_payment(id, &OkGl { journal: Uuid::new_v4(), post: Uuid::new_v4() }).await.unwrap();
    let (ps2, st2): (String, String) = sqlx::query_as("SELECT posting_state::text, status::text FROM payment.payment_entries WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(ps2, "posted");
    assert_eq!(st2, "posted");
}

// IP-2: a non-IDR payment is refused at post time; no mis-valued post reaches the ledger.
#[tokio::test]
async fn non_idr_refused_at_post() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone());
    let id = w.create_payment(receive(Uuid::new_v4(), Some("USD".into()))).await.unwrap();
    let e = w.post_payment(id, &OkGl { journal: Uuid::new_v4(), post: Uuid::new_v4() }).await.unwrap_err();
    assert!(matches!(e, PaymentError::UnsupportedCurrency(c) if c == "USD"));
}

// IP-3: the seam event is emitted EXACTLY once under a concurrent double-post — the pending→posted
// gate stops a double `PaymentSettled` that would draw an invoice's outstanding down twice via
// billing::apply_settlement. (Proactively applied from billing's council finding.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_post_emits_settled_once() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = Arc::new(PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone())));
    let id = w.create_payment(receive(Uuid::new_v4(), None)).await.unwrap();
    let gl = BarrierGl { gate: Arc::new(tokio::sync::Barrier::new(2)), journal: Uuid::new_v4(), post: Uuid::new_v4() };
    let (w1, w2, g1, g2) = (w.clone(), w.clone(), gl.clone(), gl.clone());
    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { w1.post_payment(id, &g1).await }),
        tokio::spawn(async move { w2.post_payment(id, &g2).await }),
    );
    r1.unwrap().unwrap();
    r2.unwrap().unwrap();
    let emitted = rec.events.lock().unwrap().iter().filter(|e| matches!(e, PaymentEvent::PaymentSettled(s) if s.payment_id == id)).count();
    assert_eq!(emitted, 1, "the settlement event must fire exactly once, even under a concurrent double-post");
}

// ── guarded HTTP surface: tenancy ────────────────────────────────────────────

const SECRET: &[u8] = b"payment-integrity-probe-secret";

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    company_id: Option<Uuid>,
}

/// Mint an HS256 access token. `company_id = None` models a token that authenticates a user but
/// carries no tenant — it must not be allowed to move money.
fn token(company_id: Option<Uuid>) -> String {
    let claims = TestClaims { sub: "probe-user".into(), exp: 9_999_999_999, company_id };
    encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(SECRET)).unwrap()
}

async fn module(pool: &PgPool) -> PaymentModule {
    PaymentModule::builder().with_database(pool.clone()).build().unwrap()
}
fn app(pool: &PgPool, m: &PaymentModule) -> axum::Router {
    create_guarded_payment_routes(m, pool.clone(), CompanyVerifier::hs256(SECRET))
}

/// Send a request with an optional bearer token.
async fn req_with(
    app: axum::Router,
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
    let resp = app.oneshot(builder.body(b).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
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
        number, smuggled, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(),
    )
}

// IGT-1: an unauthenticated write is rejected. Before the tenant guard this create succeeded and
// stamped whatever `companyId` the caller put in the body.
#[tokio::test]
async fn guarded_write_rejects_unauthenticated() {
    let pool = pool().await;
    let m = module(&pool).await;
    let (status, _) =
        req_with(app(&pool, &m), "POST", "/payment-entries", Some(payment_body(&uq("PE"), None)), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an unauthenticated write must not reach the service");
}

// IGT-2: a token that authenticates a user but carries no `company_id` claim is rejected — a writer
// that cannot name its tenant must never run.
#[tokio::test]
async fn guarded_write_rejects_token_without_company_id() {
    let pool = pool().await;
    let m = module(&pool).await;
    let (status, _) = req_with(
        app(&pool, &m), "POST", "/payment-entries", Some(payment_body(&uq("PE"), None)), Some(token(None)),
    ).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a token with no tenant must not write");
}

// IGT-3: a `companyId` smuggled in the body is ignored — the persisted tenant is the token's. This is
// the regression that motivated the change: the body must not be able to name the company whose money
// moves.
#[tokio::test]
async fn body_company_id_cannot_override_the_token_tenant() {
    let pool = pool().await;
    let m = module(&pool).await;
    let token_company = Uuid::new_v4();
    let attacker_company = Uuid::new_v4();
    let number = uq("PE");
    let (status, body) = req_with(
        app(&pool, &m),
        "POST",
        "/payment-entries",
        Some(payment_body(&number, Some(attacker_company))),
        Some(token(Some(token_company))),
    ).await;
    assert_eq!(status, StatusCode::CREATED, "got: {body}");

    let persisted: Uuid =
        sqlx::query_scalar("SELECT company_id FROM payment.payment_entries WHERE payment_number = $1")
            .bind(&number)
            .fetch_one(&pool)
            .await
            .expect("payment row");
    assert_eq!(persisted, token_company, "tenant must come from the token, not the body");
    assert_ne!(persisted, attacker_company, "the body's companyId must be ignored");
}
