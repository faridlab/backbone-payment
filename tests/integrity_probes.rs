//! Integrity probes for payment — invariants that must hold against a REAL Postgres beyond the
//! golden math. Requires DATABASE_URL (:5433/backbone_payment).

use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

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
