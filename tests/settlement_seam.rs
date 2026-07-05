//! The order-to-cash SETTLEMENT seam, end-to-end across THREE modules: **billing → payment →
//! accounting → billing** — closing the cash loop. Zero normal Cargo edges (billing + accounting are
//! dev-deps only).
//!
//! Flow: billing posts a Sales Invoice (Dr A/R · Cr Revenue) into the REAL ledger → outstanding =
//! grand. A partial payment (receive) posts Dr Bank · Cr A/R into the ledger + emits `PaymentSettled`;
//! an ACL routes it → billing `apply_settlement` → outstanding drawn down, schedules advanced
//! fill-in-order, status → partially_paid. A second payment settles the rest → status paid. All three
//! schemas co-locate in one DB. Requires DATABASE_URL (:5433/backbone_payment).

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use backbone_billing::application::service::billing_gl::{
    AccountingPostEnvelope as BillEnv, GlPostAck as BillAck, GlPostRejected as BillRej, GlPostSink as BillSink,
};
use backbone_billing::application::service::billing_write_service::{
    BillingWriteService, NewInvoiceLine, NewSalesInvoice,
};

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope as PayEnv, GlPostAck as PayAck, GlPostRejected as PayRej, GlPostSink as PaySink,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentWriteService,
};

use backbone_accounting::application::service::posting_service::{PostingLine, PostingRequest, PostingService};

/// ACL: either producer's serialized envelope → accounting's PostingRequest against the REAL ledger.
struct GlAdapter { svc: PostingService }
impl GlAdapter {
    #[allow(clippy::too_many_arguments)]
    async fn post_common(
        &self, company_id: Uuid, source_type: &str, source_id: Uuid, source_reference: Option<String>,
        posting_date: chrono::NaiveDate, posting_type: &str, reverses_post_id: Option<Uuid>, lines: Vec<PostingLine>,
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
impl BillSink for GlAdapter {
    async fn post(&self, e: &BillEnv) -> Result<BillAck, BillRej> {
        let lines = e.lines.iter().map(|l| PostingLine {
            account_id: l.account_id, debit: l.debit, credit: l.credit,
            party_type: l.party_type.clone(), party_id: l.party_id,
            cost_center_id: None, project_id: None, department_id: None, description: l.description.clone(),
        }).collect();
        match self.post_common(e.company_id, &e.source_type, e.source_id, e.source_reference.clone(), e.posting_date, &e.posting_type, None, lines).await {
            Ok((post_id, journal_id, idempotent_reuse)) => Ok(BillAck { post_id, journal_id, idempotent_reuse }),
            Err((code, message)) => Err(BillRej { code, message }),
        }
    }
}
#[async_trait::async_trait]
impl PaySink for GlAdapter {
    async fn post(&self, e: &PayEnv) -> Result<PayAck, PayRej> {
        let lines = e.lines.iter().map(|l| PostingLine {
            account_id: l.account_id, debit: l.debit, credit: l.credit,
            party_type: l.party_type.clone(), party_id: l.party_id,
            cost_center_id: None, project_id: None, department_id: None, description: l.description.clone(),
        }).collect();
        match self.post_common(e.company_id, &e.source_type, e.source_id, e.source_reference.clone(), e.posting_date, &e.posting_type, e.reverses_post_id, lines).await {
            Ok((post_id, journal_id, idempotent_reuse)) => Ok(PayAck { post_id, journal_id, idempotent_reuse }),
            Err((code, message)) => Err(PayRej { code, message }),
        }
    }
}

/// Records payment events so the test can route `PaymentSettled` → billing.
#[derive(Default, Clone)]
struct RecordingPaySink { events: Arc<std::sync::Mutex<Vec<PaymentEvent>>> }
impl PaymentEventSink for RecordingPaySink {
    fn publish(&self, e: PaymentEvent) { self.events.lock().unwrap().push(e); }
}

fn d(s: &str) -> Decimal { Decimal::from_str_exact(s).unwrap() }
fn day() -> chrono::NaiveDate { chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap() }
fn due(n: u32) -> chrono::NaiveDate { chrono::NaiveDate::from_ymd_opt(2026, 8, n).unwrap() }
fn uq(p: &str) -> String { format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..8]) }
async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5433/backbone_payment".to_string());
    PgPool::connect(&url).await.expect("connect DB")
}
async fn seed_coa(pool: &PgPool) -> (Uuid, HashMap<&'static str, Uuid>) {
    let company = Uuid::new_v4();
    let coa: &[(&str, &str, &str, &str, &str)] = &[
        ("1200", "Piutang Usaha", "asset", "accounts_receivable", "debit"),
        ("4000", "Pendapatan", "revenue", "operating_revenue", "credit"),
        ("1110", "Bank BCA", "asset", "bank", "debit"),
    ];
    let mut m = HashMap::new();
    for (code, name, at, st, nb) in coa {
        let id = Uuid::new_v4();
        sqlx::query(r#"INSERT INTO accounting.accounts (id, company_id, account_number, account_code, name, account_type, account_subtype, normal_balance, is_header, is_detail, status)
            VALUES ($1,$2,$3,$4,$5,$6::account_type,$7::account_subtype,$8::normal_balance,false,true,'active'::account_status)"#)
            .bind(id).bind(company).bind(code).bind(code).bind(name).bind(at).bind(st).bind(nb)
            .execute(pool).await.expect("seed acct");
        m.insert(*code, id);
    }
    (company, m)
}
async fn journal_totals(pool: &PgPool, jid: Uuid) -> (Decimal, Decimal) {
    let r = sqlx::query("SELECT total_debit, total_credit FROM accounting.journals WHERE id=$1").bind(jid).fetch_one(pool).await.unwrap();
    (r.get("total_debit"), r.get("total_credit"))
}
async fn invoice_row(pool: &PgPool, id: Uuid) -> (Decimal, String) {
    let r = sqlx::query("SELECT outstanding_amount, status::text AS st FROM billing.sales_invoices WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
    (r.get("outstanding_amount"), r.get("st"))
}

/// SSEAM-1: order-to-cash settlement across billing, payment, and the real ledger (partial → full).
#[tokio::test]
async fn settlement_across_three_modules() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let recorder = RecordingPaySink::default();
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()));
    let gl = GlAdapter { svc: PostingService::new(pool.clone()) };

    // 1) billing: Sales Invoice 1 × 1,000,000 (no tax), two installments 600k + 400k, then post.
    let inv = billing.create_sales_invoice(NewSalesInvoice {
        invoice_number: uq("SI"), company_id: company, branch_id: None, customer_id: customer, source_so_id: None,
        posting_date: day(), due_date: None, currency: None, receivable_account_id: coa["1200"],
        lines: vec![NewInvoiceLine { item_id: item, account_id: coa["4000"], description: None, quantity: d("1"), unit_price: d("1000000") }],
        tax_lines: vec![],
    }).await.unwrap();
    billing.add_payment_schedule(inv, "sales", company, &[(due(1), d("600000")), (due(15), d("400000"))]).await.unwrap();
    let inv_post = billing.post_sales_invoice(inv, &gl).await.unwrap();
    assert_eq!(journal_totals(&pool, inv_post.journal_id).await, (d("1000000"), d("1000000")));
    assert_eq!(invoice_row(&pool, inv).await, (d("1000000.00"), "submitted".to_string()));

    // 2) payment A: receive 600,000, allocate to the invoice, post → Dr Bank · Cr A/R into the ledger.
    let pay_a = payment.create_payment(NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
        party_type: Some("customer".into()), party_id: Some(customer), posting_date: day(), currency: None,
        mode_of_payment_id: None, bank_account_id: coa["1110"], party_account_id: coa["1200"], paid_amount: d("600000"),
        reference_no: None, allocations: vec![NewAllocation { invoice_ref: inv, invoice_kind: "sales".into(), amount: d("600000") }],
    }).await.unwrap();
    let pa = payment.post_payment(pay_a, &gl).await.unwrap();
    assert_eq!(journal_totals(&pool, pa.journal_id).await, (d("600000"), d("600000")));

    // 3) ACL: PaymentSettled → billing.apply_settlement (drawdown).
    apply_settlements(&billing, &recorder, pay_a).await;
    assert_eq!(invoice_row(&pool, inv).await, (d("400000.00"), "partially_paid".to_string()), "partial settlement");
    // fill-in-order: installment 1 (600k) paid, installment 2 (400k) untouched.
    let s1 = sched(&pool, inv, 1).await; assert_eq!(s1, (d("600000.00"), "paid".to_string()));
    let s2 = sched(&pool, inv, 2).await; assert_eq!(s2, (d("0.00"), "unpaid".to_string()));

    // 4) payment B: receive the remaining 400,000, settle → invoice fully paid.
    let pay_b = payment.create_payment(NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
        party_type: Some("customer".into()), party_id: Some(customer), posting_date: day(), currency: None,
        mode_of_payment_id: None, bank_account_id: coa["1110"], party_account_id: coa["1200"], paid_amount: d("400000"),
        reference_no: None, allocations: vec![NewAllocation { invoice_ref: inv, invoice_kind: "sales".into(), amount: d("400000") }],
    }).await.unwrap();
    payment.post_payment(pay_b, &gl).await.unwrap();
    apply_settlements(&billing, &recorder, pay_b).await;

    assert_eq!(invoice_row(&pool, inv).await, (d("0.00"), "paid".to_string()), "fully settled");
    assert_eq!(sched(&pool, inv, 2).await, (d("400000.00"), "paid".to_string()));
}

/// Route each `PaymentSettled` allocation → billing.apply_settlement (CLAMP). Returns the total
/// on-account remainder (`Σ requested − applied`) — cash that landed as a party credit, not stranded.
async fn apply_settlements(billing: &BillingWriteService, rec: &RecordingPaySink, payment_id: Uuid) -> Decimal {
    let evts = rec.events.lock().unwrap().clone();
    let settled = evts.iter().find_map(|e| match e {
        PaymentEvent::PaymentSettled(s) if s.payment_id == payment_id => Some(s.clone()), _ => None,
    }).expect("PaymentSettled for our payment");
    let mut on_account = Decimal::ZERO;
    for a in &settled.allocations {
        let applied = billing.apply_settlement(a.invoice_ref, &a.invoice_kind, a.amount).await.unwrap();
        on_account += a.amount - applied;
    }
    on_account
}
async fn ar_party_credit(pool: &PgPool, account: Uuid, party: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT COALESCE(SUM(credit_amount),0) - COALESCE(SUM(debit_amount),0) FROM accounting.ledgers WHERE account_id=$1 AND party_id=$2")
        .bind(account).bind(party).fetch_one(pool).await.unwrap()
}
/// Route each `PaymentCancelled` allocation → billing.reverse_settlement (restore outstanding).
async fn reverse_settlements(billing: &BillingWriteService, rec: &RecordingPaySink, payment_id: Uuid) {
    let evts = rec.events.lock().unwrap().clone();
    let cancelled = evts.iter().find_map(|e| match e {
        PaymentEvent::PaymentCancelled(c) if c.payment_id == payment_id => Some(c.clone()), _ => None,
    }).expect("PaymentCancelled for our payment");
    for a in &cancelled.allocations {
        billing.reverse_settlement(a.invoice_ref, &a.invoice_kind, a.amount).await.unwrap();
    }
}
async fn reversal_post_count(pool: &PgPool, source_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM accounting.accounting_posts WHERE source_id=$1 AND posting_type='reversal'::posting_type AND posting_status='posted'::posting_status")
        .bind(source_id).fetch_one(pool).await.unwrap()
}

/// SSEAM-3 (completeness council 2026-07-05): the refund/reversal KEEP flow. A posted payment that
/// settled an invoice to `paid` is reversed — the sign-flipped `posting_type="reversal"` journal
/// hits the ledger, `PaymentCancelled` → `reverse_settlement` restores the invoice's outstanding, and
/// the payment goes `cancelled`. A re-reverse posts once (accounting dedups) and restores once (the
/// posted→cancelled gate), so outstanding is never double-restored. This is the exit an operator
/// needs instead of hand-editing posted GL.
#[tokio::test]
async fn reverse_payment_restores_invoice_and_is_idempotent() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let recorder = RecordingPaySink::default();
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(recorder.clone()));
    let gl = GlAdapter { svc: PostingService::new(pool.clone()) };

    // Invoice 1,000,000, posted; a receive settles it fully to `paid`.
    let inv = billing.create_sales_invoice(NewSalesInvoice {
        invoice_number: uq("SI"), company_id: company, branch_id: None, customer_id: customer, source_so_id: None,
        posting_date: day(), due_date: None, currency: None, receivable_account_id: coa["1200"],
        lines: vec![NewInvoiceLine { item_id: item, account_id: coa["4000"], description: None, quantity: d("1"), unit_price: d("1000000") }],
        tax_lines: vec![],
    }).await.unwrap();
    billing.post_sales_invoice(inv, &gl).await.unwrap();
    let pay = payment.create_payment(NewPayment {
        payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
        party_type: Some("customer".into()), party_id: Some(customer), posting_date: day(), currency: None,
        mode_of_payment_id: None, bank_account_id: coa["1110"], party_account_id: coa["1200"], paid_amount: d("1000000"),
        reference_no: None, allocations: vec![NewAllocation { invoice_ref: inv, invoice_kind: "sales".into(), amount: d("1000000") }],
    }).await.unwrap();
    payment.post_payment(pay, &gl).await.unwrap();
    apply_settlements(&billing, &recorder, pay).await;
    assert_eq!(invoice_row(&pool, inv).await, (d("0.00"), "paid".to_string()));

    // Reverse the payment → reversal journal + PaymentCancelled + reverse_settlement.
    payment.reverse_payment(pay, &gl).await.unwrap();
    reverse_settlements(&billing, &recorder, pay).await;

    // Payment cancelled; invoice re-owed; the reversal journal is a real `reversal` post.
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT status::text FROM payment.payment_entries WHERE id=$1").bind(pay).fetch_one(&pool).await.unwrap(), "cancelled");
    assert_eq!(invoice_row(&pool, inv).await, (d("1000000.00"), "submitted".to_string()), "outstanding restored");
    assert_eq!(reversal_post_count(&pool, pay).await, 1);
    // Ledger: invoice Dr 1M, payment Cr 1M, reversal Dr 1M → customer net owes 1M again.
    assert_eq!(ar_party_credit(&pool, coa["1200"], customer).await, d("-1000000.00"));

    // Re-reverse: single reversal post (accounting dedups), PaymentCancelled emitted once (gate),
    // outstanding NOT double-restored.
    payment.reverse_payment(pay, &gl).await.unwrap();
    let cancelled_events = recorder.events.lock().unwrap().iter().filter(|e| matches!(e, PaymentEvent::PaymentCancelled(c) if c.payment_id == pay)).count();
    assert_eq!(cancelled_events, 1, "PaymentCancelled emitted exactly once across two reverses");
    assert_eq!(reversal_post_count(&pool, pay).await, 1, "one reversal post, not two");
    assert_eq!(invoice_row(&pool, inv).await, (d("1000000.00"), "submitted".to_string()), "outstanding not double-restored");
}

/// SSEAM-2 (council 2026-07-05, skeptic): the split invariant COMPOSES — two payments racing the same
/// invoice keep the GL A/R control and the billing subledger in agreement. Two 600k receipts each
/// allocate 600k to a 1,000,000 invoice: both post (A/R credited 1,200,000); the first apply draws it
/// to 400k, the second CLAMPS to the remaining 400k (returns applied=400k) → invoice paid, and the
/// 200k over-payment is a retrievable on-account party credit — never stranded. Without CLAMP the
/// second apply rejected and 600k vanished, diverging GL from the subledger by 600k.
#[tokio::test]
async fn racing_payments_reconcile_via_clamp_and_on_account() {
    let pool = pool().await;
    let (company, coa) = seed_coa(&pool).await;
    let customer = Uuid::new_v4();
    let item = Uuid::new_v4();

    let billing = BillingWriteService::new(pool.clone());
    let payment = PaymentWriteService::with_sink(pool.clone(), Arc::new(RecordingPaySink::default()));
    let gl = GlAdapter { svc: PostingService::new(pool.clone()) };

    // Invoice 1,000,000, posted → A/R debited 1,000,000 [customer].
    let inv = billing.create_sales_invoice(NewSalesInvoice {
        invoice_number: uq("SI"), company_id: company, branch_id: None, customer_id: customer, source_so_id: None,
        posting_date: day(), due_date: None, currency: None, receivable_account_id: coa["1200"],
        lines: vec![NewInvoiceLine { item_id: item, account_id: coa["4000"], description: None, quantity: d("1"), unit_price: d("1000000") }],
        tax_lines: vec![],
    }).await.unwrap();
    billing.post_sales_invoice(inv, &gl).await.unwrap();

    // Two independent 600,000 receipts, each allocating 600,000 to the SAME invoice.
    let mut applied_second = d("-1");
    for i in 0..2 {
        let pay = payment.create_payment(NewPayment {
            payment_number: uq("PE"), company_id: company, branch_id: None, payment_type: "receive".into(),
            party_type: Some("customer".into()), party_id: Some(customer), posting_date: day(), currency: None,
            mode_of_payment_id: None, bank_account_id: coa["1110"], party_account_id: coa["1200"], paid_amount: d("600000"),
            reference_no: None, allocations: vec![NewAllocation { invoice_ref: inv, invoice_kind: "sales".into(), amount: d("600000") }],
        }).await.unwrap();
        payment.post_payment(pay, &gl).await.unwrap();
        // apply directly (each payment settled 600k) — capture the second's clamped return.
        let a = billing.apply_settlement(inv, "sales", d("600000")).await.unwrap();
        if i == 1 { applied_second = a; }
    }

    // Second settlement clamped to the remaining 400,000.
    assert_eq!(applied_second, d("400000.00"));
    // Invoice fully paid.
    assert_eq!(invoice_row(&pool, inv).await, (d("0.00"), "paid".to_string()));
    // Reconciliation: A/R credited 1,200,000 by the two receipts, debited 1,000,000 by the invoice →
    // a 200,000 party credit balance = the on-account over-payment. GL ties to the subledger.
    assert_eq!(ar_party_credit(&pool, coa["1200"], customer).await, d("200000.00"),
        "the 200k over-payment is a retrievable on-account party credit, not stranded");
}
async fn sched(pool: &PgPool, inv: Uuid, no: i32) -> (Decimal, String) {
    let r = sqlx::query("SELECT paid_amount, status::text AS st FROM billing.payment_schedules WHERE invoice_ref=$1 AND installment_no=$2").bind(inv).bind(no).fetch_one(pool).await.unwrap();
    (r.get("paid_amount"), r.get("st"))
}
