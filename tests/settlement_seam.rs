//! The order-to-cash SETTLEMENT seam, end-to-end across THREE modules: **billing → payment →
//! accounting → billing** — closing the cash loop. Zero normal Cargo edges (billing + accounting are
//! dev-deps only).
//!
//! Flow: billing posts a Sales Invoice (Dr A/R · Cr Revenue) into the REAL ledger → outstanding =
//! grand. A partial payment (receive) posts Dr Bank · Cr A/R into the ledger + emits `PaymentSettled`;
//! an ACL routes it → billing `apply_settlement` → outstanding drawn down, schedules advanced
//! fill-in-order, status → partially_paid — AND the settlement's reconciliation-graph edge lands in
//! the same transaction (through the in-test `AccountingReconcileSink`, the composing host's shape),
//! so `outstanding == grand_total − Σ settlement edges` holds at every step. A second payment
//! settles the rest → status paid + a full-reconcile group stamps both A/R lines. All three schemas
//! co-locate in one DB. Requires DATABASE_URL (:5433/backbone_payment).

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
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

use backbone_billing::application::service::billing_gl::{
    ReconcileEdgeAck, ReconcileLine, ReconcileOrigin, ReconcilePairRequest, ReconcileRejected,
    ReconcileSink, UnreconcilePairRequest,
};
use backbone_billing::application::service::billing_write_service::{
    BillingWriteService, NewInvoiceLine, NewSalesInvoice,
};

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope as PayEnv, GlPostAck as PayAck, GlPostRejected as PayRej,
    GlPostSink as PaySink,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentWriteService,
};

use backbone_accounting::application::service::posting_service::{
    PostingLine, PostingRequest, PostingService,
};
use backbone_accounting::application::service::reconcile_write_service::ReconcileWriteService;
use backbone_accounting::domain::reconcile_graph::{LineLocator, PairRequest};
use backbone_accounting::infrastructure::persistence::{
    SqlxPostingRepository, SqlxReconcileGraphRepository,
};

/// ACL: either producer's serialized envelope → accounting's PostingRequest against the REAL ledger.
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

/// ACL: the reconciliation port over accounting's write service — the composing host's
/// implementation shape (the same adapter billing's seam tests use).
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

/// Records payment events so the test can route `PaymentSettled` → billing.
#[derive(Default, Clone)]
struct RecordingPaySink {
    events: Arc<std::sync::Mutex<Vec<PaymentEvent>>>,
}
impl PaymentEventSink for RecordingPaySink {
    fn publish(&self, e: PaymentEvent) {
        self.events.lock().unwrap().push(e);
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
    chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
}
fn due(n: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, n).unwrap()
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
        sqlx::query(r#"INSERT INTO accounting.accounts (id, account_number, account_code, name, account_type, account_subtype, normal_balance, is_header, is_detail, is_reconcilable, status)
            VALUES ($1,$2,$3,$4,$5::account_type,$6::account_subtype,$7::normal_balance,false,true,$8,'active'::account_status)"#)
            .bind(id).bind(code).bind(code).bind(name).bind(at).bind(st).bind(nb).bind(rec)
            .execute(pool).await.expect("seed acct");
        m.insert(*code, id);
    }
    (company, m)
}
async fn journal_totals(pool: &PgPool, jid: Uuid) -> (Decimal, Decimal) {
    let r = sqlx::query("SELECT total_debit, total_credit FROM accounting.journals WHERE id=$1")
        .bind(jid)
        .fetch_one(pool)
        .await
        .unwrap();
    (r.get("total_debit"), r.get("total_credit"))
}
async fn invoice_row(pool: &PgPool, id: Uuid) -> (Decimal, String) {
    let r = sqlx::query(
        "SELECT outstanding_amount, status::text AS st FROM billing.sales_invoices WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    (r.get("outstanding_amount"), r.get("st"))
}
async fn sched(pool: &PgPool, inv: Uuid, no: i32) -> (Decimal, String) {
    let r = sqlx::query("SELECT paid_amount, status::text AS st FROM billing.payment_schedules WHERE invoice_ref=$1 AND installment_no=$2").bind(inv).bind(no).fetch_one(pool).await.unwrap();
    (r.get("paid_amount"), r.get("st"))
}

// --- reconciliation-graph reads (the ledger-side proof of every settlement) ------------------
//
// The account pin + the non-reversing journal pin both matter: a payment posts Bank + A/R lines all
// stamped with the same source identity (only the A/R control carries edges), and a reversed
// payment adds a sign-flipped mirror journal with the SAME source identity (the original line is
// the non-reversing one) — same disambiguation the graph's own line locators make.

/// The test's COA account ids — the per-test separator on the tenant-agnostic accounting tables
/// (ADR-0029): every journal line this test posts references one of these freshly seeded ids, so
/// parallel tests in the shared scratch database never cross-count.
fn coa_ids(coa: &HashMap<&str, Uuid>) -> Vec<Uuid> {
    coa.values().copied().collect()
}
/// Σ settlement edges + their count for this test's accounts.
async fn settlement_edges(pool: &PgPool, accounts: &[Uuid]) -> (Decimal, i64) {
    sqlx::query_as("SELECT COALESCE(SUM(amount),0), COUNT(*) FROM accounting.partial_reconciles WHERE origin='settlement' \
                    AND (debit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)) \
                      OR credit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)))")
        .bind(accounts).fetch_one(pool).await.unwrap()
}
/// The control-account ORIGINAL (non-reversing) line's residual — its signed amount minus every
/// partial touching it. This is the authoritative "still owed / still unapplied" read.
/// Isolation comes from the account + source ids, both freshly minted per test.
async fn residual(
    pool: &PgPool,
    _company: Uuid,
    source_type: &str,
    source_id: Uuid,
    account: Uuid,
) -> Decimal {
    sqlx::query_scalar(
        r#"SELECT (CASE WHEN l.base_debit_amount > 0 THEN l.base_debit_amount ELSE l.base_credit_amount END)
                 - COALESCE((SELECT SUM(p.amount) FROM accounting.partial_reconciles p WHERE p.debit_move_id=l.id),0)
                 - COALESCE((SELECT SUM(p.amount) FROM accounting.partial_reconciles p WHERE p.credit_move_id=l.id),0)
             FROM accounting.journal_lines l JOIN accounting.journals j ON j.id=l.journal_id
            WHERE l.source_type=$1 AND l.source_id=$2 AND l.account_id=$3
              AND l.is_posted AND j.is_reversing=false"#,
    )
    .bind(source_type).bind(source_id).bind(account)
    .fetch_one(pool).await.unwrap()
}
async fn line_reconciled(
    pool: &PgPool,
    _company: Uuid,
    source_type: &str,
    source_id: Uuid,
    account: Uuid,
) -> (bool, Option<Uuid>) {
    sqlx::query_as(
        "SELECT l.is_reconciled, l.full_reconcile_id FROM accounting.journal_lines l JOIN accounting.journals j ON j.id=l.journal_id \
         WHERE l.source_type=$1 AND l.source_id=$2 AND l.account_id=$3 AND l.is_posted AND j.is_reversing=false",
    )
    .bind(source_type).bind(source_id).bind(account)
    .fetch_one(pool).await.unwrap()
}
async fn full_groups(pool: &PgPool, accounts: &[Uuid]) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM accounting.full_reconciles WHERE id IN \
                       (SELECT full_reconcile_id FROM accounting.journal_lines WHERE account_id = ANY($1) AND full_reconcile_id IS NOT NULL)")
        .bind(accounts)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// SSEAM-1: order-to-cash settlement across billing, payment, and the real ledger (partial → full),
/// with the reconciliation graph proving every step: one settlement edge per payment, the full
/// group stamping all three A/R lines when the invoice closes.
#[tokio::test]
async fn settlement_across_three_modules() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let sink = AccountingReconcileSink::new(&pool);
    let recorder = RecordingPaySink::default();
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()))
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let gl = GlAdapter { svc: PostingService::new(Arc::new(backbone_accounting::infrastructure::persistence::posting_repository::SqlxPostingRepository::new(pool.clone()))) };

    // 1) billing: Sales Invoice 1 × 1,000,000 (no tax), two installments 600k + 400k, then post.
    let inv = billing
        .create_sales_invoice(NewSalesInvoice {
            invoice_number: uq("SI"),
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
    billing
        .add_payment_schedule(
            inv,
            "sales",
            company,
            &[(due(1), d("600000")), (due(15), d("400000"))],
        )
        .await
        .unwrap();
    let inv_post = billing.post_sales_invoice(inv, &gl).await.unwrap();
    assert_eq!(
        journal_totals(&pool, inv_post.journal_id).await,
        (d("1000000"), d("1000000"))
    );
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("1000000.00"), "submitted".to_string())
    );
    // Nothing reconciled yet: no edges, the A/R debit fully open.
    assert_eq!(settlement_edges(&pool, &coa_ids(&coa)).await, (Decimal::ZERO, 0));
    assert_eq!(
        residual(&pool, company, "order", inv, coa["1200"]).await,
        d("1000000.00")
    );

    // 2) payment A: receive 600,000, allocate to the invoice, post → Dr Bank · Cr A/R into the ledger.
    let pay_a = payment
        .create_payment(NewPayment {
            payment_number: uq("PE"),
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
    let pa = in_org_scope(&pool, payment.post_payment(pay_a, &gl)).await.unwrap();
    assert_eq!(
        journal_totals(&pool, pa.journal_id).await,
        (d("600000"), d("600000"))
    );

    // 3) ACL: PaymentSettled → billing.apply_settlement (drawdown + graph edge, one transaction).
    apply_settlements(&billing, &sink, &recorder, pay_a).await;
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("400000.00"), "partially_paid".to_string()),
        "partial settlement"
    );
    // fill-in-order: installment 1 (600k) paid, installment 2 (400k) untouched.
    let s1 = sched(&pool, inv, 1).await;
    assert_eq!(s1, (d("600000.00"), "paid".to_string()));
    let s2 = sched(&pool, inv, 2).await;
    assert_eq!(s2, (d("0.00"), "unpaid".to_string()));
    // Graph: exactly one settlement edge of 600k. The payment's credit line is fully consumed
    // (residual 0) but the INVOICE line still carries 400k — partial, so the component is not
    // all-zero: NO full group yet and no line flagged.
    assert_eq!(settlement_edges(&pool, &coa_ids(&coa)).await, (d("600000.00"), 1));
    assert_eq!(
        residual(&pool, company, "order", inv, coa["1200"]).await,
        d("400000.00")
    );
    assert_eq!(
        residual(&pool, company, "payment", pay_a, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(full_groups(&pool, &coa_ids(&coa)).await, 0);
    // The seam's invariant: outstanding == grand_total − Σ settlement edges.
    assert_eq!(
        d("400000.00"),
        d("1000000.00") - settlement_edges(&pool, &coa_ids(&coa)).await.0
    );

    // 4) payment B: receive the remaining 400,000, settle → invoice fully paid + full reconcile.
    let pay_b = payment
        .create_payment(NewPayment {
            payment_number: uq("PE"),
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
            paid_amount: d("400000"),
            reference_no: None,
            allocations: vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("400000"),
            }],
            withholding_amount: rust_decimal::Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();
    in_org_scope(&pool, payment.post_payment(pay_b, &gl)).await.unwrap();
    apply_settlements(&billing, &sink, &recorder, pay_b).await;

    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("0.00"), "paid".to_string()),
        "fully settled"
    );
    assert_eq!(
        sched(&pool, inv, 2).await,
        (d("400000.00"), "paid".to_string())
    );
    // Graph: one edge per payment (600k + 400k); the invoice line and BOTH payment lines are
    // consumed into ONE full-reconcile group — every A/R line of the chain stamped + flagged.
    let (sum, n) = settlement_edges(&pool, &coa_ids(&coa)).await;
    assert_eq!(
        (sum, n),
        (d("1000000.00"), 2),
        "one settlement edge per payment"
    );
    assert_eq!(
        residual(&pool, company, "order", inv, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "payment", pay_a, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "payment", pay_b, coa["1200"]).await,
        Decimal::ZERO
    );
    let (inv_rec, inv_full) = line_reconciled(&pool, company, "order", inv, coa["1200"]).await;
    assert!(inv_rec, "invoice A/R line reconciled");
    let group = inv_full.expect("group id stamped");
    let (ra, fa) = line_reconciled(&pool, company, "payment", pay_a, coa["1200"]).await;
    assert!(ra);
    assert_eq!(fa, Some(group), "both payment lines join the SAME group");
    assert_eq!(full_groups(&pool, &coa_ids(&coa)).await, 1);
}

/// Route each `PaymentSettled` allocation → billing.apply_settlement (CLAMP + graph edge). Returns
/// the total on-account remainder (`Σ requested − applied`) — cash that landed as a party credit.
async fn apply_settlements(
    billing: &BillingWriteService,
    sink: &AccountingReconcileSink,
    rec: &RecordingPaySink,
    payment_id: Uuid,
) -> Decimal {
    let evts = rec.events.lock().unwrap().clone();
    let settled = evts
        .iter()
        .find_map(|e| match e {
            PaymentEvent::PaymentSettled(s) if s.payment_id == payment_id => Some(s.clone()),
            _ => None,
        })
        .expect("PaymentSettled for our payment");
    let mut on_account = Decimal::ZERO;
    for a in &settled.allocations {
        let applied = billing
            .apply_settlement(
                settled.company_id,
                a.invoice_ref,
                &a.invoice_kind,
                a.amount,
                payment_id,
                sink,
            )
            .await
            .unwrap()
            .applied;
        on_account += a.amount - applied;
    }
    on_account
}
async fn ar_party_credit(pool: &PgPool, account: Uuid, party: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT COALESCE(SUM(credit_amount),0) - COALESCE(SUM(debit_amount),0) FROM accounting.ledgers WHERE account_id=$1 AND party_id=$2")
        .bind(account).bind(party).fetch_one(pool).await.unwrap()
}
/// Route each `PaymentCancelled` allocation → billing.reverse_settlement (unlink edge + restore).
async fn reverse_settlements(
    billing: &BillingWriteService,
    sink: &AccountingReconcileSink,
    rec: &RecordingPaySink,
    payment_id: Uuid,
) {
    let evts = rec.events.lock().unwrap().clone();
    let cancelled = evts
        .iter()
        .find_map(|e| match e {
            PaymentEvent::PaymentCancelled(c) if c.payment_id == payment_id => Some(c.clone()),
            _ => None,
        })
        .expect("PaymentCancelled for our payment");
    for a in &cancelled.allocations {
        billing
            .reverse_settlement(
                cancelled.company_id,
                a.invoice_ref,
                &a.invoice_kind,
                a.amount,
                payment_id,
                sink,
            )
            .await
            .unwrap();
    }
}
async fn reversal_post_count(pool: &PgPool, source_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM accounting.accounting_posts WHERE source_id=$1 AND posting_type='reversal'::posting_type AND posting_status='posted'::posting_status")
        .bind(source_id).fetch_one(pool).await.unwrap()
}

/// SSEAM-3 (completeness council 2026-07-05): the refund/reversal KEEP flow, proven to net zero on
/// BOTH the ledger and the graph. A posted payment that settled an invoice to `paid` is reversed —
/// the sign-flipped `posting_type="reversal"` journal hits the ledger, `PaymentCancelled` →
/// `reverse_settlement` unlinks the settlement edge (dissolving the full group, clearing the
/// invoice line's flags), and the graph pairs the payment's original A/R credit with the reversal's
/// A/R debit (reverse-then-reconcile) — so the payment's own lines also net zero. A re-reverse
/// posts once (accounting dedups) and restores once (the posted→cancelled gate). This is the exit
/// an operator needs instead of hand-editing posted GL.
#[tokio::test]
async fn reverse_payment_restores_invoice_and_is_idempotent() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let sink = AccountingReconcileSink::new(&pool);
    let recorder = RecordingPaySink::default();
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()))
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let gl = GlAdapter { svc: PostingService::new(Arc::new(backbone_accounting::infrastructure::persistence::posting_repository::SqlxPostingRepository::new(pool.clone()))) };

    // Invoice 1,000,000, posted; a receive settles it fully to `paid`.
    let inv = billing
        .create_sales_invoice(NewSalesInvoice {
            invoice_number: uq("SI"),
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
    let pay = payment
        .create_payment(NewPayment {
            payment_number: uq("PE"),
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
            paid_amount: d("1000000"),
            reference_no: None,
            allocations: vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("1000000"),
            }],
            withholding_amount: rust_decimal::Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();
    in_org_scope(&pool, payment.post_payment(pay, &gl)).await.unwrap();
    apply_settlements(&billing, &sink, &recorder, pay).await;
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("0.00"), "paid".to_string())
    );
    // Fully reconciled before the reverse: one settlement edge, one group, both lines flagged.
    assert_eq!(settlement_edges(&pool, &coa_ids(&coa)).await, (d("1000000.00"), 1));
    assert_eq!(full_groups(&pool, &coa_ids(&coa)).await, 1);
    assert!(
        line_reconciled(&pool, company, "order", inv, coa["1200"])
            .await
            .0
    );

    // Reverse the payment → reversal journal + PaymentCancelled → reverse_settlement.
    in_org_scope(&pool, payment.reverse_payment(pay, &gl)).await.unwrap();
    reverse_settlements(&billing, &sink, &recorder, pay).await;

    // Payment cancelled; invoice re-owed; the reversal journal is a real `reversal` post.
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM payment.payment_entries WHERE id=$1"
        )
        .bind(pay)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "cancelled"
    );
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("1000000.00"), "submitted".to_string()),
        "outstanding restored"
    );
    assert_eq!(reversal_post_count(&pool, pay).await, 1);
    // Ledger: invoice Dr 1M, payment Cr 1M, reversal Dr 1M → customer net owes 1M again.
    assert_eq!(
        ar_party_credit(&pool, coa["1200"], customer).await,
        d("-1000000.00")
    );
    // Graph nets zero too: the settlement edge is GONE (origin='settlement' count 0) and the
    // invoice's A/R line is back to fully open, unflagged, with no full group of its own — the
    // group that survives is the reverse-then-reconcile pairing of the payment's original credit
    // with the reversal's debit (the payment's own two lines netting zero).
    assert_eq!(
        settlement_edges(&pool, &coa_ids(&coa)).await,
        (Decimal::ZERO, 0),
        "the settlement edge is unlinked"
    );
    assert_eq!(
        residual(&pool, company, "order", inv, coa["1200"]).await,
        d("1000000.00"),
        "invoice A/R reopened"
    );
    let (inv_rec, inv_full) = line_reconciled(&pool, company, "order", inv, coa["1200"]).await;
    assert!(!inv_rec, "invoice line flag cleared");
    assert!(inv_full.is_none());
    let (pay_rec, pay_full) = line_reconciled(&pool, company, "payment", pay, coa["1200"]).await;
    assert!(
        pay_rec,
        "payment original credit paired with the reversal debit"
    );
    assert!(
        pay_full.is_some(),
        "the reverse-then-reconcile pair forms its own group"
    );
    let paired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM accounting.partial_reconciles WHERE metadata->>'rule'='reverse_then_reconcile' \
                 AND (debit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)) \
                   OR credit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)))")
        .bind(coa_ids(&coa)).fetch_one(&pool).await.unwrap();
    assert_eq!(paired, 1, "exactly one reverse-then-reconcile edge");

    // Re-reverse: single reversal post (accounting dedups), PaymentCancelled emitted once (gate),
    // outstanding NOT double-restored — and the graph is untouched by the no-op.
    in_org_scope(&pool, payment.reverse_payment(pay, &gl)).await.unwrap();
    let cancelled_events = recorder
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PaymentEvent::PaymentCancelled(c) if c.payment_id == pay))
        .count();
    assert_eq!(
        cancelled_events, 1,
        "PaymentCancelled emitted exactly once across two reverses"
    );
    assert_eq!(
        reversal_post_count(&pool, pay).await,
        1,
        "one reversal post, not two"
    );
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("1000000.00"), "submitted".to_string()),
        "outstanding not double-restored"
    );
    assert_eq!(settlement_edges(&pool, &coa_ids(&coa)).await, (Decimal::ZERO, 0));
}

/// SSEAM-2 (council 2026-07-05, skeptic): the split invariant COMPOSES — two payments racing the same
/// invoice keep the GL A/R control and the billing subledger in agreement. Two 600k receipts each
/// allocate 600k to a 1,000,000 invoice: both post (A/R credited 1,200,000); the first apply draws it
/// to 400k, the second CLAMPS to the remaining 400k (returns applied=400k) → invoice paid, and the
/// 200k over-payment is a retrievable on-account party credit — never stranded. Without CLAMP the
/// second apply rejected and 600k vanished, diverging GL from the subledger by 600k.
///
/// Graph-side: the second edge clamps WITH the billing clamp (600k + 400k = exactly the invoice),
/// the invoice line closes into a full group, and the second payment's surviving 200k residual IS
/// the on-account credit — the ledger states it, the graph measures it.
#[tokio::test]
async fn racing_payments_reconcile_via_clamp_and_on_account() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let sink = AccountingReconcileSink::new(&pool);
    let payment =
        PaymentWriteService::with_sink(pool.clone(), Arc::new(RecordingPaySink::default()))
            .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let gl = GlAdapter { svc: PostingService::new(Arc::new(backbone_accounting::infrastructure::persistence::posting_repository::SqlxPostingRepository::new(pool.clone()))) };

    // Invoice 1,000,000, posted → A/R debited 1,000,000 [customer].
    let inv = billing
        .create_sales_invoice(NewSalesInvoice {
            invoice_number: uq("SI"),
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

    // Two independent 600,000 receipts, each allocating 600,000 to the SAME invoice.
    let mut applied_second = d("-1");
    let mut pays = Vec::new();
    for _i in 0..2 {
        let pay = payment
            .create_payment(NewPayment {
                payment_number: uq("PE"),
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
        in_org_scope(&pool, payment.post_payment(pay, &gl)).await.unwrap();
        // apply directly (each payment settled 600k) — capture the second's clamped return.
        let a = billing
            .apply_settlement(company, inv, "sales", d("600000"), pay, &sink)
            .await
            .unwrap()
            .applied;
        if _i == 1 {
            applied_second = a;
        }
        pays.push(pay);
    }
    let (pay1, pay2) = (pays[0], pays[1]);

    // Second settlement clamped to the remaining 400,000.
    assert_eq!(applied_second, d("400000.00"));
    // Invoice fully paid.
    assert_eq!(
        invoice_row(&pool, inv).await,
        (d("0.00"), "paid".to_string())
    );
    // Reconciliation: A/R credited 1,200,000 by the two receipts, debited 1,000,000 by the invoice →
    // a 200,000 party credit balance = the on-account over-payment. GL ties to the subledger.
    assert_eq!(
        ar_party_credit(&pool, coa["1200"], customer).await,
        d("200000.00"),
        "the 200k over-payment is a retrievable on-account party credit, not stranded"
    );
    // Graph: edges total exactly the invoice (600k + 400k) — the second edge clamped WITH billing —
    // so the invoice line closes into a full group while pay2's line keeps a 200k residual: the
    // on-account credit, measured by the graph itself.
    let (sum, n) = settlement_edges(&pool, &coa_ids(&coa)).await;
    assert_eq!(
        (sum, n),
        (d("1000000.00"), 2),
        "edges clamp together with the subledger"
    );
    assert_eq!(
        residual(&pool, company, "order", inv, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "payment", pay1, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "payment", pay2, coa["1200"]).await,
        d("200000.00"),
        "the racing payment's surviving residual IS the on-account credit"
    );
    // The invoice line is fully consumed but NOT flagged: its connected component still holds
    // pay2's open 200k credit, and a full group only forms when EVERY member's residual is zero —
    // the credit stays retrievable instead of being papered over by a premature group.
    assert!(
        !line_reconciled(&pool, company, "order", inv, coa["1200"])
            .await
            .0,
        "the component stays open while the on-account credit lives"
    );
    assert_eq!(full_groups(&pool, &coa_ids(&coa)).await, 0);
}

/// SSEAM-4: ONE payment, TWO invoices — every allocation writes its own edge, and the payment's
/// cancellation unwinds BOTH in the same transaction. The graph mirrors the allocation list
/// one-for-one; a reverse that only undid one invoice would strand the other's edge against a
/// restored subledger.
#[tokio::test]
async fn two_allocations_write_two_edges_and_reverse_unwinds_both() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let sink = AccountingReconcileSink::new(&pool);
    let recorder = RecordingPaySink::default();
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()))
        .with_reconcilable_port(std::sync::Arc::new(AlwaysReconcilable));
    let gl = GlAdapter { svc: PostingService::new(Arc::new(backbone_accounting::infrastructure::persistence::posting_repository::SqlxPostingRepository::new(pool.clone()))) };

    let new_invoice = |amount: Decimal| NewSalesInvoice {
        invoice_number: uq("SI"),
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
            unit_price: amount,
            tax_template_id: None,
        }],
        tax_lines: vec![],
    };
    let inv_a = billing
        .create_sales_invoice(new_invoice(d("500000")))
        .await
        .unwrap();
    let inv_b = billing
        .create_sales_invoice(new_invoice(d("400000")))
        .await
        .unwrap();
    billing.post_sales_invoice(inv_a, &gl).await.unwrap();
    billing.post_sales_invoice(inv_b, &gl).await.unwrap();

    // One 900,000 receipt split across BOTH invoices.
    let pay = payment
        .create_payment(NewPayment {
            payment_number: uq("PE"),
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
            paid_amount: d("900000"),
            reference_no: None,
            allocations: vec![
                NewAllocation {
                    invoice_ref: inv_a,
                    invoice_kind: "sales".into(),
                    amount: d("500000"),
                },
                NewAllocation {
                    invoice_ref: inv_b,
                    invoice_kind: "sales".into(),
                    amount: d("400000"),
                },
            ],
            withholding_amount: rust_decimal::Decimal::ZERO,
            withholding_account_id: None,
            withholding_tax_type: "none".into(),
        })
        .await
        .unwrap();
    in_org_scope(&pool, payment.post_payment(pay, &gl)).await.unwrap();
    let on_account = apply_settlements(&billing, &sink, &recorder, pay).await;
    assert_eq!(
        on_account,
        Decimal::ZERO,
        "nothing on-account — both allocations absorbed"
    );

    // Both invoices paid; TWO edges, one per allocation. The shared payment line connects BOTH
    // invoice lines into ONE fully-zero component → ONE full group holding all three lines
    // (the union-find sweep stamps every connected zero-residual member, not just the pair).
    assert_eq!(
        invoice_row(&pool, inv_a).await,
        (d("0.00"), "paid".to_string())
    );
    assert_eq!(
        invoice_row(&pool, inv_b).await,
        (d("0.00"), "paid".to_string())
    );
    let (sum, n) = settlement_edges(&pool, &coa_ids(&coa)).await;
    assert_eq!((sum, n), (d("900000.00"), 2), "one edge per allocation");
    assert_eq!(
        residual(&pool, company, "order", inv_a, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "order", inv_b, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        residual(&pool, company, "payment", pay, coa["1200"]).await,
        Decimal::ZERO
    );
    assert_eq!(
        full_groups(&pool, &coa_ids(&coa)).await,
        1,
        "one connected component, one group"
    );
    let (_, ga) = line_reconciled(&pool, company, "order", inv_a, coa["1200"]).await;
    let (_, gb) = line_reconciled(&pool, company, "order", inv_b, coa["1200"]).await;
    let (_, gp) = line_reconciled(&pool, company, "payment", pay, coa["1200"]).await;
    assert_eq!((ga, gb), (gp, gp), "all three lines stamp the SAME group");

    // Cancel the payment: the reversal journal hits the ledger, PaymentCancelled unwinds BOTH
    // allocations — both edges unlinked, both outstandings restored, both groups gone (the only
    // surviving group pairs the payment's own credit with its reversal debit).
    in_org_scope(&pool, payment.reverse_payment(pay, &gl)).await.unwrap();
    reverse_settlements(&billing, &sink, &recorder, pay).await;
    assert_eq!(
        invoice_row(&pool, inv_a).await,
        (d("500000.00"), "submitted".to_string()),
        "A restored"
    );
    assert_eq!(
        invoice_row(&pool, inv_b).await,
        (d("400000.00"), "submitted".to_string()),
        "B restored"
    );
    assert_eq!(
        settlement_edges(&pool, &coa_ids(&coa)).await,
        (Decimal::ZERO, 0),
        "both edges unlinked"
    );
    assert_eq!(
        residual(&pool, company, "order", inv_a, coa["1200"]).await,
        d("500000.00")
    );
    assert_eq!(
        residual(&pool, company, "order", inv_b, coa["1200"]).await,
        d("400000.00")
    );
    assert!(
        !line_reconciled(&pool, company, "order", inv_a, coa["1200"])
            .await
            .0
    );
    assert!(
        !line_reconciled(&pool, company, "order", inv_b, coa["1200"])
            .await
            .0
    );
    let paired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM accounting.partial_reconciles WHERE metadata->>'rule'='reverse_then_reconcile' \
                 AND (debit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)) \
                   OR credit_move_id IN (SELECT id FROM accounting.journal_lines WHERE account_id = ANY($1)))")
        .bind(coa_ids(&coa)).fetch_one(&pool).await.unwrap();
    assert_eq!(
        paired, 1,
        "the payment's own credit↔reversal pair is the only survivor"
    );
    // Ledger nets zero for the whole round trip: Dr A/R (invoices) 900k, Cr A/R 900k, Dr 900k back.
    assert_eq!(
        ar_party_credit(&pool, coa["1200"], customer).await,
        d("-900000.00")
    );
}
