//! CCC — the bank-confirmation drift consumer (`confirm_cash_once`), the ONLY writer of
//! in_flight→paid. Exactly-once over an at-least-once bus: the inbox dedup and the CAS drift commit
//! in ONE transaction, so a redelivery is a no-op and a crash between dedup and drift commits
//! neither. A lost event strands the status at in_flight — a stuck LABEL, never wrong money: the
//! GL and the knock-offs committed with the post. Requires DATABASE_URL (:5433/backbone_payment)
//! with `backbone_outbox::outbox::migrate` applied for the payment schema.

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

#[derive(Default, Clone)]
struct OkGl;
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        assert!(
            env.is_balanced(),
            "payment emitted an UNBALANCED post: {env:?}"
        );
        Ok(GlPostAck {
            post_id: Uuid::new_v4(),
            journal_id: Uuid::new_v4(),
            idempotent_reuse: false,
        })
    }
}

const CONSUMER: &str = "cash-confirm-probe";

// Serialize the outbox bootstrap across this binary's concurrent tests: its policy stanza is
// DROP-then-CREATE, which two racing calls can interleave into "already exists".
static MIGRATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn migrate_outbox(pool: &PgPool) {
    let _guard = MIGRATE.lock().await;
    backbone_outbox::outbox::migrate(pool, "payment")
        .await
        .unwrap();
}

fn svc(pool: &PgPool, rec: Recorder) -> PaymentWriteService {
    PaymentWriteService::with_sink(pool.clone(), Arc::new(rec))
        .with_reconcilable_port(Arc::new(AlwaysReconcilable))
}

async fn new_in_flight_payment(pool: &PgPool, company: Uuid, w: &PaymentWriteService) -> Uuid {
    let id = w
        .create_payment(NewPayment {
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
                invoice_ref: Uuid::new_v4(),
                invoice_kind: "sales".into(),
                amount: d("100000"),
            }],
            withholding_amount: Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();
    w.post_payment(id, &OkGl).await.unwrap();
    id
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

// CCC-1: the first delivery drifts in_flight→paid; an at-least-once REDELIVERY of the same event id
// is a no-op (the inbox deduped it) — the exactly-once core.
#[tokio::test]
async fn first_delivery_drifts_and_redelivery_no_ops() {
    let pool = pool().await;
    migrate_outbox(&pool).await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());
    let id = new_in_flight_payment(&pool, company, &w).await;
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    let event = Uuid::new_v4();
    let drifted = w
        .confirm_cash_once(event, CONSUMER, company, id)
        .await
        .unwrap();
    assert!(drifted, "the first delivery performs the drift");
    assert_eq!(status_of(&pool, id).await, ("paid".into(), "posted".into()));

    // Redelivery of the SAME event: consumed already, no-op, no error.
    let again = w
        .confirm_cash_once(event, CONSUMER, company, id)
        .await
        .unwrap();
    assert!(!again, "a redelivered event is a no-op");
    assert_eq!(status_of(&pool, id).await, ("paid".into(), "posted".into()));

    assert!(
        backbone_outbox::inbox::was_consumed(&pool, "payment", CONSUMER, event)
            .await
            .unwrap()
    );
}

// CCC-2: an event naming a payment in a NON-applicable state (still draft — the event outran the
// post) is consumed but drifts nothing, and reports false. It is never an error: the bus may
// legitimately carry events for payments that have not landed yet.
#[tokio::test]
async fn non_applicable_state_consumes_without_drift() {
    let pool = pool().await;
    migrate_outbox(&pool).await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());

    let id = w
        .create_payment(NewPayment {
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
                invoice_ref: Uuid::new_v4(),
                invoice_kind: "sales".into(),
                amount: d("100000"),
            }],
            withholding_amount: Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();

    let event = Uuid::new_v4();
    let drifted = w
        .confirm_cash_once(event, CONSUMER, company, id)
        .await
        .unwrap();
    assert!(!drifted, "a draft payment is not driftable");
    assert_eq!(
        status_of(&pool, id).await,
        ("draft".into(), "pending".into())
    );
    // The event was still CONSUMED — a replay will not drift it later either; the re-drift command
    // is the documented recovery for an event that outran its payment.
    assert!(
        backbone_outbox::inbox::was_consumed(&pool, "payment", CONSUMER, event)
            .await
            .unwrap()
    );
}

// CCC-3: a lost event strands the status at in_flight — the documented stuck LABEL. The invariant
// that makes it safe: the GL synced when the post ran (posting_state='posted', journal present),
// so the money is never wrong, only the label is stale until the re-drift command replays the
// consumer.
#[tokio::test]
async fn lost_event_strands_the_label_not_the_money() {
    let pool = pool().await;
    migrate_outbox(&pool).await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());
    let id = new_in_flight_payment(&pool, company, &w).await;

    // No event ever arrives. The stuck state: status in_flight, but the GL synced.
    let (status, posting_state): (String, String) = sqlx::query_as(
        "SELECT status::text, posting_state::text FROM payment.payment_entries WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), posting_state.as_str()),
        ("in_flight", "posted")
    );
    let journal: Option<Uuid> =
        sqlx::query_scalar("SELECT journal_id FROM payment.payment_entries WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        journal.is_some(),
        "the journal committed with the post — only the label waits"
    );

    // Recovery: the re-drift is the SAME consumer under a new event id — it drifts exactly once.
    let redrift = w
        .confirm_cash_once(Uuid::new_v4(), CONSUMER, company, id)
        .await
        .unwrap();
    assert!(redrift, "the re-drift command performs the drift");
    assert_eq!(status_of(&pool, id).await, ("paid".into(), "posted".into()));
}

// CCC-4: drift isolation — one event drifts only the payment it names; another in_flight payment
// for the same company is untouched.
#[tokio::test]
async fn drift_is_scoped_to_the_named_payment() {
    let pool = pool().await;
    migrate_outbox(&pool).await;
    let company = Uuid::new_v4();
    let w = svc(&pool, Recorder::default());
    let a = new_in_flight_payment(&pool, company, &w).await;
    let b = new_in_flight_payment(&pool, company, &w).await;

    let drifted = w
        .confirm_cash_once(Uuid::new_v4(), CONSUMER, company, a)
        .await
        .unwrap();
    assert!(drifted);
    assert_eq!(status_of(&pool, a).await, ("paid".into(), "posted".into()));
    assert_eq!(
        status_of(&pool, b).await,
        ("in_flight".into(), "posted".into()),
        "the sibling payment still waits for its own confirmation"
    );
}
