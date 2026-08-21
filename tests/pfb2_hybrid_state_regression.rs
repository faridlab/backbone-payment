//! PFB2 — the hybrid-state trap guard. The fused state machine is only as safe as its WRITER SET:
//! verbs write status through CAS only, the bank-confirmation consumer is the ONLY in_flight→paid
//! writer, and no HTTP route may PATCH a status. These probes pin the trap so a future change that
//! adds a fifth writer fails HERE first. Requires DATABASE_URL (:5433/backbone_payment).

use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostRejected, GlPostSink,
};
use backbone_payment::application::service::payment_lifecycle::BankReconcilablePort;
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};

/// Tests inject the reconcilability read — the real probe reads accounting, which these fixtures do
/// not populate; landing-state behavior is asserted via the stub.
struct AlwaysReconcilable;
#[async_trait::async_trait]
impl BankReconcilablePort for AlwaysReconcilable {
    async fn bank_reconcilable(
        &self,
        _pool: &sqlx::PgPool,
        _company_id: uuid::Uuid,
        _account_id: uuid::Uuid,
    ) -> Result<bool, PaymentError> {
        Ok(true)
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
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

#[derive(Default, Clone)]
struct Recorder {
    events: Arc<Mutex<Vec<PaymentEvent>>>,
}
impl PaymentEventSink for Recorder {
    fn publish(&self, e: PaymentEvent) {
        self.events.lock().unwrap().push(e);
    }
}

/// A GL sink that RECORDS but never acks-refuses — the probe asserts whether it was driven at all.
#[derive(Default, Clone)]
struct CountingGl {
    seen: Arc<Mutex<Vec<AccountingPostEnvelope>>>,
}
#[async_trait::async_trait]
impl GlPostSink for CountingGl {
    async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        assert!(
            env.is_balanced(),
            "payment emitted an UNBALANCED post: {env:?}"
        );
        self.seen.lock().unwrap().push(env.clone());
        Ok(GlPostAck {
            post_id: Uuid::new_v4(),
            journal_id: Uuid::new_v4(),
            idempotent_reuse: false,
        })
    }
}

fn svc(pool: &PgPool, rec: Recorder) -> PaymentWriteService {
    PaymentWriteService::with_sink(pool.clone(), Arc::new(rec))
        .with_reconcilable_port(Arc::new(AlwaysReconcilable))
}

fn draft_payment(company: Uuid, inv: Uuid) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"),
        company_id: company,
        branch_id: None,
        payment_type: "receive".into(),
        party_type: Some("customer".into()),
        party_id: Some(Uuid::new_v4()),
        posting_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 21).unwrap(),
        currency: None,
        mode_of_payment_id: None,
        method: None,
        provider_txn_id: None,
        bank_account_id: Uuid::new_v4(),
        party_account_id: Uuid::new_v4(),
        paid_amount: d("100000"),
        reference_no: None,
        allocations: vec![NewAllocation {
            invoice_ref: inv,
            invoice_kind: "sales".into(),
            amount: d("100000"),
        }],
        withholding_amount: Decimal::ZERO,
        withholding_account_id: None,
        withholding_tax_type: "none".into(),
    }
}

async fn status_of(pool: &PgPool, id: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT status::text, posting_state::text FROM payment.payment_entries WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// PFB2-1: posting a REJECTED payment refuses BEFORE the GL sink is driven. The CAS alone would
// refuse only after the journal existed — a rejected payment would leave a post its own row
// disowns. The pre-sink status gate is the guard; this probe is its tripwire.
#[tokio::test]
async fn post_on_rejected_payment_refuses_before_the_sink() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());
    let id = w
        .create_payment(draft_payment(company, Uuid::new_v4()))
        .await
        .unwrap();
    w.submit_payment(company, id).await.unwrap();
    w.reject_payment(company, id).await.unwrap();

    let gl = CountingGl::default();
    match w.post_payment(id, &gl).await.unwrap_err() {
        PaymentError::NotPostable(s) => assert_eq!(s, "rejected"),
        e => panic!("expected NotPostable, got {e:?}"),
    }
    assert!(
        gl.seen.lock().unwrap().is_empty(),
        "no journal may exist for a terminal payment"
    );
    assert_eq!(
        status_of(&pool, id).await,
        ("rejected".into(), "pending".into())
    );
}

// PFB2-2: once landed in_flight, NO hand verb reaches paid. Submit refuses; reject refuses (a
// landed payment's exit is reverse — reject is the pre-settlement stop); reverse goes to
// cancelled. The only in_flight→paid writer is the
// bank-confirmation consumer (pinned in `cash_confirm_consumer_suite`); a re-post is an idempotent
// reuse that never re-lands.
#[tokio::test]
async fn no_hand_verb_reaches_paid_from_in_flight() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());
    let id = w
        .create_payment(draft_payment(company, Uuid::new_v4()))
        .await
        .unwrap();
    w.post_payment(id, &CountingGl::default()).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    // submit refuses (its only arm is draft→submitted).
    match w.submit_payment(company, id).await.unwrap_err() {
        PaymentError::NotSubmittable(s) => assert_eq!(s, "in_flight"),
        e => panic!("expected NotSubmittable, got {e:?}"),
    }

    // A re-post is the idempotent short-circuit — same journal, no re-landing, no second event.
    let gl = CountingGl::default();
    let out = w.post_payment(id, &gl).await.unwrap();
    assert!(
        out.idempotent_reuse,
        "a landed payment re-posts as an idempotent reuse"
    );
    assert_eq!(
        gl.seen.lock().unwrap().len(),
        0,
        "the sink is not re-driven for an already-landed payment"
    );
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    // reverse leaves for cancelled — not paid.
    w.reverse_payment(id, &CountingGl::default()).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("cancelled".into(), "posted".into())
    );
}

// PFB2-3: the HTTP write surface exposes the verbs but NO status write. A PATCH (or PUT) to the
// payment-entry resource — even fully authenticated — matches no route at all (404, not 405/422):
// the only writers are the mounted verbs, exactly as the route table documents. The probe mints a
// real HS256 token so the discriminator cannot be "unauthenticated".
#[tokio::test]
async fn no_route_patches_status() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let pool = pool().await;
    let company = Uuid::new_v4();
    let m = backbone_payment::PaymentModule::builder()
        .with_database(pool.clone())
        .build()
        .expect("module");
    let secret = b"pfb2-probe-secret";
    let verifier = backbone_auth::company::CompanyVerifier::hs256(secret);

    // A real draft payment to aim at — so a 404 can only mean "no route", never "no entity".
    let w = svc(&pool, Recorder::default());
    let id = w
        .create_payment(draft_payment(company, Uuid::new_v4()))
        .await
        .unwrap();

    // Mint the token the guard accepts (same claims shape the composing service issues).
    #[derive(serde::Serialize)]
    struct Claims {
        sub: String,
        exp: usize,
        company_id: Uuid,
    }
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &Claims {
            sub: "probe".into(),
            exp: (chrono::Utc::now().timestamp() + 600) as usize,
            company_id: company,
        },
        &jsonwebtoken::EncodingKey::from_secret(secret),
    )
    .unwrap();

    let router = backbone_payment::presentation::http::create_guarded_payment_routes(
        &m,
        pool.clone(),
        verifier,
    );

    // Control: the mounted verb IS reachable through the same router + token (204 = the draft→submitted arm).
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/payment-entries/{id}/submit"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "control: the submit verb route exists and the token is valid"
    );
    assert_eq!(
        status_of(&pool, id).await,
        ("submitted".into(), "pending".into())
    );

    // Probes: status writes must match NO route (404), with the same valid token.
    for (method, uri) in [
        ("PATCH", format!("/payment-entries/{id}")),
        ("PATCH", format!("/payment-entries/{id}/status")),
        ("PUT", format!("/payment-entries/{id}")),
    ] {
        let body = serde_json::json!({ "status": "paid" }).to_string();
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri.clone())
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} {uri}: a status write must match no route (404), got {}",
            resp.status()
        );
    }

    // And the payment is untouched by the probes.
    assert_eq!(
        status_of(&pool, id).await,
        ("submitted".into(), "pending".into())
    );
}
