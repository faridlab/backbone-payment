//! Golden oracle for the payment write path (settlement intent). Payment-only — the cash-loop seam
//! into the real ledger + billing is proven in `settlement_seam.rs`. Posting here uses a FAKE
//! `GlPostSink` (deterministic ack) so the math + state machine are tested in isolation.
//! Requires DATABASE_URL (:5433/backbone_payment).

use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
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

/// Fake GL sink: records the envelope, asserts balance, returns a fixed ack.
#[derive(Default, Clone)]
struct FakeGl { seen: Arc<Mutex<Vec<AccountingPostEnvelope>>>, journal: Uuid, post: Uuid }
impl FakeGl {
    fn new() -> Self { Self { seen: Arc::new(Mutex::new(Vec::new())), journal: Uuid::new_v4(), post: Uuid::new_v4() } }
    fn last(&self) -> AccountingPostEnvelope { self.seen.lock().unwrap().last().unwrap().clone() }
}
#[async_trait::async_trait]
impl GlPostSink for FakeGl {
    async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        assert!(env.is_balanced(), "payment emitted an UNBALANCED post: {env:?}");
        self.seen.lock().unwrap().push(env.clone());
        Ok(GlPostAck { post_id: self.post, journal_id: self.journal, idempotent_reuse: false })
    }
}
#[derive(Default, Clone)]
struct Recorder { events: Arc<Mutex<Vec<PaymentEvent>>> }
impl PaymentEventSink for Recorder {
    fn publish(&self, e: PaymentEvent) { self.events.lock().unwrap().push(e); }
}

fn alloc(inv: Uuid, kind: &str, amt: &str) -> NewAllocation {
    NewAllocation { invoice_ref: inv, invoice_kind: kind.into(), amount: d(amt) }
}
fn receive(company: Uuid, bank: Uuid, ar: Uuid, customer: Uuid, paid: &str, allocs: Vec<NewAllocation>) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
        party_type: Some("customer".into()), party_id: Some(customer), posting_date: day(), currency: None,
        mode_of_payment_id: None, bank_account_id: bank, party_account_id: ar, paid_amount: d(paid),
        reference_no: None, allocations: allocs,
    }
}

// PGC-1: receive math + post — receive 1,000,000, allocate 600,000 to one invoice → allocated 600,000,
// unallocated 400,000; post Dr Bank 1,000,000 · Cr A/R 1,000,000 [customer] (balanced).
#[tokio::test]
async fn receive_math_and_post() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone()));
    let (company, bank, ar, customer, inv) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let id = w.create_payment(receive(company, bank, ar, customer, "1000000", vec![alloc(inv, "sales", "600000")])).await.unwrap();

    let r = sqlx::query("SELECT paid_amount, allocated_amount, unallocated_amount, status::text AS st FROM payment.payment_entries WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(r.get::<Decimal, _>("paid_amount"), d("1000000.00"));
    assert_eq!(r.get::<Decimal, _>("allocated_amount"), d("600000.00"));
    assert_eq!(r.get::<Decimal, _>("unallocated_amount"), d("400000.00"));
    assert_eq!(r.get::<String, _>("st"), "draft");

    let gl = FakeGl::new();
    w.post_payment(id, &gl).await.unwrap();
    let env = gl.last();
    assert_eq!(env.totals(), (d("1000000.00"), d("1000000.00")));
    let ar_line = env.lines.iter().find(|l| l.account_id == ar).unwrap();
    assert_eq!(ar_line.credit, d("1000000.00"));
    assert_eq!(ar_line.party_type.as_deref(), Some("customer"));
    let bank_line = env.lines.iter().find(|l| l.account_id == bank).unwrap();
    assert_eq!(bank_line.debit, d("1000000.00"));

    // events: one PaymentSettled (1 allocation) + one PaymentReceivedOnAccount (400,000 remainder).
    let evts = rec.events.lock().unwrap().clone();
    let settled = evts.iter().find_map(|e| match e { PaymentEvent::PaymentSettled(s) if s.payment_id == id => Some(s.clone()), _ => None }).expect("PaymentSettled");
    assert_eq!(settled.allocations.len(), 1);
    assert_eq!(settled.allocations[0].amount, d("600000.00"));
    assert_eq!(settled.paid_amount, d("1000000.00"));
    assert!(evts.iter().any(|e| matches!(e, PaymentEvent::PaymentReceivedOnAccount(o) if o.unallocated_amount == d("400000.00"))));
}

// PGC-2: pay math + post — pay a supplier 500,000 → Dr A/P 500,000 [supplier] · Cr Bank 500,000.
#[tokio::test]
async fn pay_supplier_post() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone());
    let (company, bank, ap, supplier, inv) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let p = NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "pay".into(),
        party_type: Some("supplier".into()), party_id: Some(supplier), posting_date: day(), currency: None,
        mode_of_payment_id: None, bank_account_id: bank, party_account_id: ap, paid_amount: d("500000"),
        reference_no: None, allocations: vec![alloc(inv, "purchase", "500000")],
    };
    let id = w.create_payment(p).await.unwrap();
    let gl = FakeGl::new();
    w.post_payment(id, &gl).await.unwrap();
    let env = gl.last();
    assert_eq!(env.totals(), (d("500000.00"), d("500000.00")));
    let ap_line = env.lines.iter().find(|l| l.account_id == ap).unwrap();
    assert_eq!(ap_line.debit, d("500000.00"));
    assert_eq!(ap_line.party_type.as_deref(), Some("supplier"));
    assert_eq!(env.lines.iter().find(|l| l.account_id == bank).unwrap().credit, d("500000.00"));
}

// PGC-3: posting is idempotent — a second post reuses the ack, hits the sink once, emits nothing new.
#[tokio::test]
async fn posting_is_idempotent() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone()));
    let (company, bank, ar, customer, inv) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let id = w.create_payment(receive(company, bank, ar, customer, "300000", vec![alloc(inv, "sales", "300000")])).await.unwrap();
    let gl = FakeGl::new();
    let first = w.post_payment(id, &gl).await.unwrap();
    assert!(!first.idempotent_reuse);
    let second = w.post_payment(id, &gl).await.unwrap();
    assert!(second.idempotent_reuse);
    assert_eq!(first.journal_id, second.journal_id);
    assert_eq!(gl.seen.lock().unwrap().len(), 1, "the sink is hit exactly once");
    let settled = rec.events.lock().unwrap().iter().filter(|e| matches!(e, PaymentEvent::PaymentSettled(s) if s.payment_id == id)).count();
    assert_eq!(settled, 1, "PaymentSettled emitted exactly once across two posts");
}

// PGC-4: validation gates — over-allocation, non-positive amount, duplicate number.
#[tokio::test]
async fn validation_gates() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone());
    let (company, bank, ar, customer, inv) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    // over-allocation: allocate 700,000 against a 500,000 receipt.
    let e = w.create_payment(receive(company, bank, ar, customer, "500000", vec![alloc(inv, "sales", "700000")])).await.unwrap_err();
    assert!(matches!(e, PaymentError::OverAllocated { .. }));
    // non-positive paid amount.
    let e = w.create_payment(receive(company, bank, ar, customer, "0", vec![])).await.unwrap_err();
    assert!(matches!(e, PaymentError::NonPositiveAmount));
    // duplicate number.
    let mut p = receive(company, bank, ar, customer, "100000", vec![]);
    let num = uq("DUP"); p.payment_number = num.clone();
    w.create_payment(p.clone()).await.unwrap();
    p.payment_number = num;
    assert!(matches!(w.create_payment(p).await.unwrap_err(), PaymentError::DuplicateNumber(_)));
}

// PGC-5: a fully-allocated receive emits NO on-account event (unallocated == 0).
#[tokio::test]
async fn fully_allocated_no_on_account() {
    let pool = pool().await;
    let rec = Recorder::default();
    let w = PaymentWriteService::with_sink(pool.clone(), Arc::new(rec.clone()));
    let (company, bank, ar, customer, inv) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let id = w.create_payment(receive(company, bank, ar, customer, "250000", vec![alloc(inv, "sales", "250000")])).await.unwrap();
    let un: Decimal = sqlx::query_scalar("SELECT unallocated_amount FROM payment.payment_entries WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(un, d("0.00"));
    w.post_payment(id, &FakeGl::new()).await.unwrap();
    assert!(!rec.events.lock().unwrap().iter().any(|e| matches!(e, PaymentEvent::PaymentReceivedOnAccount(_))), "no remainder → no on-account event");
}
