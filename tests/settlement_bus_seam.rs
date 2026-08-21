//! The settlement seam, run through the GO-LIVE DURABLE BUS (backbone-outbox): **payment → outbox →
//! relay → inbox → REAL billing settlement consumer → reconciliation graph**. This proves the three
//! properties every module's council parked for go-live, on the real payment→billing money seam:
//!
//!   (a) CRASH-SAFE: the `PaymentSettled` event is staged in the outbox; a crash between the settlement
//!       and delivery loses nothing — the relay delivers it whenever it next runs.
//!   (b) EXACTLY-ONCE under at-least-once redelivery: a redelivered `PaymentSettled` is deduped at
//!       billing's inbox, so the settlement draws the invoice down — AND writes its reconciliation
//!       edge — EXACTLY ONCE, closing the parked "no redelivery dedup → double-draw" gap.
//!   (c) WIRE CONTRACT: payment's outbox payload deserializes straight into billing's
//!       `PaymentSettledDto` mirror — the event JSON is the contract (zero payment Cargo edge), and
//!       drift here fails the consume, not the books.
//!
//! Zero normal Cargo edges: billing/accounting/outbox are dev-deps only. Requires
//! DATABASE_URL (:5433/backbone_payment) with billing + accounting schemas co-located.

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use backbone_accounting::application::service::posting_service::{
    PostingLine, PostingRequest, PostingService,
};
use backbone_accounting::application::service::reconcile_write_service::ReconcileWriteService;
use backbone_accounting::domain::reconcile_graph::{LineLocator, PairRequest};
use backbone_accounting::infrastructure::persistence::{
    SqlxPostingRepository, SqlxReconcileGraphRepository,
};
use backbone_billing::application::service::billing_write_service::{
    BillingWriteService, NewInvoiceLine, NewSalesInvoice,
};
use backbone_billing::application::service::settlement_consumer::{
    PaymentSettledDto, PaymentSettledHandler,
};
use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope as PayEnv, GlPostAck as PayAck, GlPostRejected as PayRej,
    GlPostSink as PaySink, ReconcileEdgeAck, ReconcileLine, ReconcileOrigin, ReconcilePairRequest,
    ReconcileRejected, ReconcileSink, UnreconcilePairRequest,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentWriteService,
};

use backbone_outbox::{inbox, outbox, relay, OutboxError, OutboxRecord};

// --- ACL over the REAL ledger (both producers' envelopes → accounting PostingService) ---
struct GlAdapter {
    svc: PostingService,
}
impl GlAdapter {
    #[allow(clippy::too_many_arguments)]
    async fn post_common(
        &self,
        company_id: Uuid,
        source_type: &str,
        source_id: Uuid,
        source_reference: Option<String>,
        posting_date: chrono::NaiveDate,
        posting_type: &str,
        reverses_post_id: Option<Uuid>,
        lines: Vec<PostingLine>,
    ) -> Result<(Uuid, Uuid, bool), (String, String)> {
        let mut r = PostingRequest::original(company_id, source_type, source_id, posting_date);
        r.source_reference = source_reference;
        r.posting_type = posting_type.to_string();
        r.reverses_post_id = reverses_post_id;
        r.lines = lines;
        match self.svc.post(r, None).await {
            Ok(x) => Ok((x.post_id, x.journal_id, x.idempotent_reuse)),
            Err(x) => Err((x.code().to_string(), x.to_string())),
        }
    }
}
#[async_trait::async_trait]
impl PaySink for GlAdapter {
    async fn post(&self, e: &PayEnv) -> Result<PayAck, PayRej> {
        let lines = e
            .lines
            .iter()
            .map(|l| PostingLine {
                account_id: l.account_id,
                debit: l.debit,
                credit: l.credit,
                party_type: l.party_type.clone(),
                party_id: l.party_id,
                cost_center_id: None,
                project_id: None,
                department_id: None,
                description: l.description.clone(),
            })
            .collect();
        match self
            .post_common(
                e.company_id,
                &e.source_type,
                e.source_id,
                e.source_reference.clone(),
                e.posting_date,
                &e.posting_type,
                e.reverses_post_id,
                lines,
            )
            .await
        {
            Ok((post_id, journal_id, idempotent_reuse)) => Ok(PayAck {
                post_id,
                journal_id,
                idempotent_reuse,
            }),
            Err((code, message)) => Err(PayRej { code, message }),
        }
    }
}

#[derive(Default, Clone)]
struct RecordingPaySink {
    events: Arc<std::sync::Mutex<Vec<PaymentEvent>>>,
}
impl PaymentEventSink for RecordingPaySink {
    fn publish(&self, e: PaymentEvent) {
        self.events.lock().unwrap().push(e);
    }
}

/// ACL: the reconciliation port over accounting's write service — the composing host's shape. The
/// trait is implemented through PAYMENT's own `payment_gl` re-export and consumed as billing's —
/// both name the one shared `backbone-gl-posting` contract, which is the point of the re-export.
struct AccountingReconcileSink {
    svc: ReconcileWriteService,
}
impl AccountingReconcileSink {
    fn new(pool: &PgPool) -> Self {
        Self {
            svc: ReconcileWriteService::new(
                Arc::new(SqlxReconcileGraphRepository::new()),
                Arc::new(SqlxPostingRepository::new(pool.clone())),
                pool.clone(),
                None,
            ),
        }
    }
}
#[async_trait::async_trait]
impl ReconcileSink for AccountingReconcileSink {
    async fn reconcile_pair_on(
        &self,
        conn: &mut sqlx::PgConnection,
        req: &ReconcilePairRequest,
    ) -> Result<ReconcileEdgeAck, ReconcileRejected> {
        let origin = match req.origin {
            ReconcileOrigin::Settlement => "settlement",
            ReconcileOrigin::Clearing => "clearing",
            ReconcileOrigin::Manual => "manual",
        };
        let to_loc = |l: &ReconcileLine| LineLocator {
            source_type: l.source_type.clone(),
            source_id: l.source_id,
            account_id: l.account_id,
            reversing: l.reversing,
        };
        match self
            .svc
            .reconcile_pair_on(
                conn,
                &PairRequest {
                    company_id: req.company_id,
                    debit: to_loc(&req.debit),
                    credit: to_loc(&req.credit),
                    amount: req.amount,
                    origin: origin.to_string(),
                    actor: None,
                },
            )
            .await
        {
            Ok(o) => Ok(ReconcileEdgeAck {
                partial_id: o.partial_id,
                applied: o.applied,
                full_reconcile_id: o.full_reconcile_id,
            }),
            Err(e) => Err(ReconcileRejected {
                code: e.code().to_string(),
                message: e.to_string(),
            }),
        }
    }
    async fn unreconcile_pair_on(
        &self,
        conn: &mut sqlx::PgConnection,
        req: &UnreconcilePairRequest,
    ) -> Result<(), ReconcileRejected> {
        let to_loc = |l: &ReconcileLine| LineLocator {
            source_type: l.source_type.clone(),
            source_id: l.source_id,
            account_id: l.account_id,
            reversing: l.reversing,
        };
        self.svc
            .unreconcile_pair_on(
                conn,
                req.company_id,
                &to_loc(&req.debit),
                &to_loc(&req.credit),
            )
            .await
            .map_err(|e| ReconcileRejected {
                code: e.code().to_string(),
                message: e.to_string(),
            })
    }
}

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
    chrono::NaiveDate::from_ymd_opt(2026, 7, 7).unwrap()
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
async fn seed_coa(pool: &PgPool) -> (Uuid, HashMap<&'static str, Uuid>) {
    let company = Uuid::new_v4();
    // (code, name, type, subtype, normal balance, is_reconcilable) — the A/R control carries the
    // settlement edges, so it must be flagged reconcilable (guard G3).
    let coa: &[(&str, &str, &str, &str, &str, bool)] = &[
        (
            "1200",
            "Piutang Usaha",
            "asset",
            "accounts_receivable",
            "debit",
            true,
        ),
        (
            "4000",
            "Pendapatan",
            "revenue",
            "operating_revenue",
            "credit",
            false,
        ),
        ("1110", "Bank BCA", "asset", "bank", "debit", false),
    ];
    let mut m = HashMap::new();
    for (code, name, at, st, nb, rec) in coa {
        let id = Uuid::new_v4();
        sqlx::query(r#"INSERT INTO accounting.accounts (id, company_id, account_number, account_code, name, account_type, account_subtype, normal_balance, is_header, is_detail, is_reconcilable, status)
            VALUES ($1,$2,$3,$4,$5,$6::account_type,$7::account_subtype,$8::normal_balance,false,true,$9,'active'::account_status)"#)
            .bind(id).bind(company).bind(code).bind(code).bind(name).bind(at).bind(st).bind(nb).bind(rec)
            .execute(pool).await.expect("seed acct");
        m.insert(*code, id);
    }
    (company, m)
}
async fn outstanding(pool: &PgPool, id: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT outstanding_amount FROM billing.sales_invoices WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
/// Σ settlement edges + their count for the company — the graph-side twin of `outstanding`.
async fn settlement_edges(pool: &PgPool, company: Uuid) -> (Decimal, i64) {
    sqlx::query_as("SELECT COALESCE(SUM(amount),0), COUNT(*) FROM accounting.partial_reconciles WHERE company_id=$1 AND origin='settlement'")
        .bind(company).fetch_one(pool).await.unwrap()
}

/// SBSEAM-1 — the settlement seam routed through the durable outbox: crash-safe + redelivery-deduped.
#[tokio::test]
async fn settlement_through_outbox_is_exactly_once() {
    let pool = pool().await;
    outbox::migrate(&pool, "payment").await.unwrap(); // producer outbox
    outbox::migrate(&pool, "billing").await.unwrap(); // consumer inbox
    let (company, coa) = seed_coa(&pool).await;
    let (customer, item) = (Uuid::new_v4(), Uuid::new_v4());

    let billing = BillingWriteService::new(pool.clone());
    let recorder = RecordingPaySink::default();
    // Producer wired to the durable outbox: post_payment stages `PaymentSettled` in the posted tx.
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()))
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable))
        .with_outbox_schema("payment");
    let gl = GlAdapter { svc: PostingService::new(Arc::new(backbone_accounting::infrastructure::persistence::posting_repository::SqlxPostingRepository::new(pool.clone()))) };

    // Invoice 1,000,000, posted → outstanding 1,000,000.
    let inv = billing
        .create_sales_invoice(NewSalesInvoice {
            invoice_number: uq("SI"),
            company_id: company,
            branch_id: None,
            customer_id: customer,
            source_so_id: None,
            posting_date: day(),
            due_date: None,
            payment_term_id: None,
            currency: None,
            receivable_account_id: coa["1200"],
            lines: vec![NewInvoiceLine {
                item_id: item,
                account_id: coa["4000"],
                description: None,
                quantity: d("1"),
                unit_price: d("1000000"),
                tax_template_id: None,
            }],
            tax_lines: vec![],
        })
        .await
        .unwrap();
    billing.post_sales_invoice(inv, &gl).await.unwrap();
    assert_eq!(outstanding(&pool, inv).await, d("1000000.00"));

    // Payment receives 600,000, posts to the ledger, emits PaymentSettled.
    let pay = payment
        .create_payment(NewPayment {
            payment_number: uq("PE"),
            company_id: company,
            branch_id: None,
            payment_type: "receive".into(),
            party_type: Some("customer".into()),
            party_id: Some(customer),
            posting_date: day(),
            currency: None,
            mode_of_payment_id: None,
            method: None,
            provider_txn_id: None,
            bank_account_id: coa["1110"],
            party_account_id: coa["1200"],
            paid_amount: d("600000"),
            reference_no: None,
            allocations: vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("600000"),
            }],
            withholding_amount: rust_decimal::Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();
    payment.post_payment(pay, &gl).await.unwrap();
    let _ = &recorder; // the legacy in-proc sink still fires; the durable path is the outbox below.

    // PRODUCER (shipped path): post_payment already staged `PaymentSettled` into payment.outbox_events,
    // atomically with the posted transition. A crash HERE loses nothing — the event is durable.
    let event_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM payment.outbox_events WHERE aggregate_id=$1 AND event_type='PaymentSettled'")
        .bind(pay.to_string()).fetch_one(&pool).await.unwrap();

    // CONSUMER handler (shipped path): billing's `PaymentSettledHandler` over its wire-mirror DTO —
    // the outbox payload deserializes straight into it, then one inbox dedup on the bus event id
    // wraps ALL allocations + the graph edge, atomic with the drawdown.
    let hpool = pool.clone();
    let consume = move |rec: OutboxRecord| {
        let pool = hpool.clone();
        async move {
            let handler = PaymentSettledHandler::new(
                Arc::new(BillingWriteService::new(pool.clone())),
                Arc::new(AccountingReconcileSink::new(&pool)),
                "settlement-consumer",
            );
            let event: PaymentSettledDto = serde_json::from_value(rec.payload.clone())
                .map_err(|e| OutboxError::Publish(format!("payment→billing wire drift: {e}")))?;
            handler
                .handle(rec.id, &event)
                .await
                .map_err(|e| OutboxError::Publish(format!("{e:?}")))?;
            Ok(())
        }
    };

    // RELAY (runs post-"crash"): delivers the staged event → invoice drawn 1,000,000 → 400,000
    // AND the settlement edge written in the same consume.
    let n = relay::drain_once(&pool, "payment", 100, &consume)
        .await
        .unwrap();
    assert_eq!(n, 1, "the durable event was delivered after the crash");
    assert_eq!(
        outstanding(&pool, inv).await,
        d("400000.00"),
        "settlement applied once via REAL billing"
    );
    assert_eq!(
        settlement_edges(&pool, company).await,
        (d("600000.00"), 1),
        "the consume also wrote the graph edge — subledger and ledger moved together"
    );

    // AT-LEAST-ONCE REDELIVERY: force the relay to hand the same event over again (as it would if it
    // crashed after publishing but before marking published). The inbox dedups it → NO double-draw
    // and NO double-edge.
    sqlx::query("UPDATE payment.outbox_events SET published_at = NULL WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .unwrap();
    let n = relay::drain_once(&pool, "payment", 100, &consume)
        .await
        .unwrap();
    assert_eq!(n, 1, "the redelivered event was drained again");
    assert_eq!(
        outstanding(&pool, inv).await,
        d("400000.00"),
        "redelivery deduped at the inbox — the settlement did NOT double-draw (exactly-once)"
    );
    assert_eq!(
        settlement_edges(&pool, company).await,
        (d("600000.00"), 1),
        "exactly-once extends to the graph — no second edge from the redelivery"
    );

    // And the inbox records it as consumed exactly once.
    assert!(
        inbox::was_consumed(&pool, "billing", "settlement-consumer", event_id)
            .await
            .unwrap()
    );
}
