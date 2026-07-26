//! ADSEAM-1 — aging + dunning across the billing↔payment seam.
//! Billing creates + posts an invoice (outstanding set); payment's dunning engine reads it via
//! `BillingReceivablesPort`, buckets by days-past-due, and escalates. Zero normal Cargo edges.

use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use backbone_billing::application::service::billing_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostRejected, GlPostSink,
};
use backbone_billing::application::service::billing_write_service::{
    BillingWriteService, NewInvoiceLine, NewSalesInvoice,
};
use backbone_payment::application::service::billing_receivables_port::{
    BillingReceivablesPort, ReceivableRow,
};
use backbone_payment::application::service::payment_dunning_service::PaymentDunningService;

fn d(s: &str) -> Decimal { Decimal::from_str_exact(s).unwrap() }
fn uq(p: &str) -> String { format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..8]) }

/// A GL sink that always acks (no real accounting needed).
struct OkGl;
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Ok(GlPostAck { post_id: Uuid::new_v4(), journal_id: Uuid::new_v4(), idempotent_reuse: false })
    }
}

/// In-test adapter: reads billing's outstanding directly (the composition ACL does this in prod).
struct BillingReceivablesAdapter { pool: PgPool }
#[async_trait::async_trait]
impl BillingReceivablesPort for BillingReceivablesAdapter {
    async fn outstanding_for(&self, company_id: Uuid, kind: &str, _as_of: NaiveDate) -> Result<Vec<ReceivableRow>, String> {
        let rows = sqlx::query(
            r#"SELECT id, customer_id, due_date, outstanding_amount
               FROM billing.sales_invoices
               WHERE company_id = $1 AND posting_state = 'posted' AND outstanding_amount > 0
                 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(company_id)
        .fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
        Ok(rows.iter().map(|r| ReceivableRow {
            invoice_ref: r.get("id"),
            invoice_kind: kind.to_string(),
            party_id: r.get("customer_id"),
            due_date: r.get("due_date"),
            outstanding_amount: r.get("outstanding_amount"),
        }).collect())
    }
}

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5433/backbone_payment".to_string());
    PgPool::connect(&url).await.expect("connect DB")
}

#[tokio::test]
async fn aging_buckets_and_dunning_escalate_across_seam() {
    let pool = pool().await;
    let today = chrono::Utc::now().date_naive();
    let due_45d_ago = today.checked_sub_days(chrono::Days::new(45)).unwrap();

    // 1) Billing: create + post an invoice due 45 days ago → outstanding 1,000,000.
    let billing = BillingWriteService::new(pool.clone());
    let (company, customer, ar, revenue, item) = (
        Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(),
    );
    let inv = billing.create_sales_invoice(NewSalesInvoice {
        invoice_number: uq("SI"), company_id: company, branch_id: None, customer_id: customer,
        source_so_id: None, posting_date: due_45d_ago, due_date: Some(due_45d_ago),
        currency: None, receivable_account_id: ar,
        lines: vec![NewInvoiceLine { item_id: item, account_id: revenue, description: None,
            quantity: d("1"), unit_price: d("1000000") }],
        tax_lines: vec![],
    }).await.unwrap();
    billing.post_sales_invoice(inv, &OkGl).await.unwrap();

    // 2) Payment: run aging via the read port (zero cargo edge).
    let port = Arc::new(BillingReceivablesAdapter { pool: pool.clone() });
    let dunning = PaymentDunningService::new(pool.clone(), port);
    let snapshot_id = dunning.run_aging_snapshot(company, "receive", today).await.unwrap();

    // 3) Assert: 45 days past due → bucket_31_60 = 1,000,000.
    let bucket_31_60: Decimal = sqlx::query_scalar(
        "SELECT bucket_31_60 FROM payment.aging_snapshots WHERE id = $1")
        .bind(snapshot_id).fetch_one(&pool).await.unwrap();
    assert_eq!(bucket_31_60, d("1000000.00"), "45 days → bucket_31_60");

    // 4) Run dunning.
    let (_run_id, emitted) = dunning.run_dunning(company, "receive", today).await.unwrap();
    assert_eq!(emitted, 1, "one dunning action emitted");

    // 5) Assert: 45 days → level "final_notice" (31–60 range).
    let level: String = sqlx::query_scalar(
        "SELECT level::text FROM payment.dunning_actions WHERE invoice_ref = $1")
        .bind(inv).fetch_one(&pool).await.unwrap();
    assert_eq!(level, "final_notice", "45 days → final_notice escalation");

    // 6) Idempotency: re-run aging → same snapshot id.
    let snapshot2 = dunning.run_aging_snapshot(company, "receive", today).await.unwrap();
    assert_eq!(snapshot_id, snapshot2, "re-run reuses the snapshot");

    // 7) Idempotency: re-run dunning → no new actions.
    let (_, emitted2) = dunning.run_dunning(company, "receive", today).await.unwrap();
    assert_eq!(emitted2, 0, "re-run emits no new actions (unique fence)");
}
