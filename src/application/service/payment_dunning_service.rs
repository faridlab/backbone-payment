//! The dunning/aging engine (hand-authored, user-owned) — the receivables-timeline read-model +
//! escalation state. Reads billing's outstanding via `BillingReceivablesPort` (zero cargo edge),
//! buckets by days-past-due, and emits dunning actions per invoice.
//!
//! Real-world rule: an invoice ages from its `due_date`; a payment allocation de-ages it.

use backbone_orm::company_scope;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use super::billing_receivables_port::{BillingReceivablesPort, ReceivableRow};

#[derive(Debug)]
pub enum PaymentDunningError {
    Port(String),
    Db(sqlx::Error),
}
impl std::fmt::Display for PaymentDunningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentDunningError::Port(e) => write!(f, "receivables port: {e}"),
            PaymentDunningError::Db(e) => write!(f, "db: {e}"),
        }
    }
}
impl std::error::Error for PaymentDunningError {}
impl From<sqlx::Error> for PaymentDunningError {
    fn from(e: sqlx::Error) -> Self { PaymentDunningError::Db(e) }
}

#[derive(Clone)]
pub struct PaymentDunningService {
    db_pool: PgPool,
    receivables: Arc<dyn BillingReceivablesPort>,
}

impl PaymentDunningService {
    pub fn new(db_pool: PgPool, receivables: Arc<dyn BillingReceivablesPort>) -> Self {
        Self { db_pool, receivables }
    }

    /// Run one aging snapshot: read outstanding, bucket by days-past-due, persist.
    /// Idempotent: the unique (company, as_of, direction) fence means a re-run for the same date
    /// reuses the snapshot.
    pub async fn run_aging_snapshot(
        &self, company_id: Uuid, direction: &str, as_of: NaiveDate,
    ) -> Result<Uuid, PaymentDunningError> {
        let recs = self.receivables
            .outstanding_for(company_id, direction, as_of)
            .await
            .map_err(PaymentDunningError::Port)?;

        let mut tx = self.db_pool.begin().await?;
        company_scope::bind_company_on(&mut tx, company_id).await?;

        // Idempotent snapshot insert (unique company + as_of + direction).
        let snapshot_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO payment.aging_snapshots (id, company_id, as_of_date, direction, status)
               VALUES ($1, $2, $3, $4, 'final'::snapshot_status)
               ON CONFLICT (company_id, as_of_date, direction) WHERE (metadata->>'deleted_at') IS NULL
               DO UPDATE SET status = 'final'::snapshot_status
               RETURNING id"#,
        )
        .bind(Uuid::new_v4()).bind(company_id).bind(as_of).bind(direction)
        .fetch_one(&mut *tx).await?;

        let mut totals = [Decimal::ZERO; 5]; // current, 1_30, 31_60, 61_90, 90p

        for r in &recs {
            let due = r.due_date.unwrap_or(as_of);
            let dpd = as_of.signed_duration_since(due).num_days();
            let idx = bucket_index(dpd);
            totals[idx] += r.outstanding_amount;

            sqlx::query(
                r#"INSERT INTO payment.aging_buckets
                     (id, snapshot_id, company_id, invoice_ref, invoice_kind, party_id,
                      due_date, days_past_due, outstanding_amount, bucket)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::aging_bucket_name)"#,
            )
            .bind(Uuid::new_v4()).bind(snapshot_id).bind(company_id)
            .bind(r.invoice_ref).bind(&r.invoice_kind).bind(r.party_id)
            .bind(due).bind(dpd as i32).bind(r.outstanding_amount)
            .bind(bucket_name(dpd))
            .execute(&mut *tx).await?;
        }

        sqlx::query(
            r#"UPDATE payment.aging_snapshots
                 SET total_outstanding = $2, bucket_current = $3, bucket_1_30 = $4,
                     bucket_31_60 = $5, bucket_61_90 = $6, bucket_90p = $7
               WHERE id = $1"#,
        )
        .bind(snapshot_id).bind(totals.iter().copied().sum::<Decimal>())
        .bind(totals[0]).bind(totals[1]).bind(totals[2]).bind(totals[3]).bind(totals[4])
        .execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(snapshot_id)
    }

    /// Run one dunning cycle: read outstanding, escalate each overdue invoice by days-past-due,
    /// emit actions (unique on invoice_ref + level → idempotent).
    pub async fn run_dunning(
        &self, company_id: Uuid, direction: &str, as_of: NaiveDate,
    ) -> Result<(Uuid, i32), PaymentDunningError> {
        let recs = self.receivables
            .outstanding_for(company_id, direction, as_of)
            .await
            .map_err(PaymentDunningError::Port)?;

        let mut tx = self.db_pool.begin().await?;
        company_scope::bind_company_on(&mut tx, company_id).await?;

        let run_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO payment.dunning_runs (id, company_id, as_of_date, direction, status)
               VALUES ($1, $2, $3, $4, 'completed'::dunning_run_status) RETURNING id"#,
        )
        .bind(Uuid::new_v4()).bind(company_id).bind(as_of).bind(direction)
        .fetch_one(&mut *tx).await?;

        let mut emitted = 0i32;
        for r in &recs {
            let due = r.due_date.unwrap_or(as_of);
            let dpd = as_of.signed_duration_since(due).num_days();
            if dpd <= 0 { continue; } // not overdue — no action
            let (level, action_type) = dunning_level(dpd);

            let inserted = sqlx::query(
                r#"INSERT INTO payment.dunning_actions
                     (id, company_id, run_id, invoice_ref, invoice_kind, party_id,
                      level, action_type, days_past_due, outstanding_amount)
                   VALUES ($1, $2, $3, $4, $5, $6, $7::dunning_level, $8::dunning_action_type, $9, $10)
                   ON CONFLICT (invoice_ref, invoice_kind, level) WHERE (metadata->>'deleted_at') IS NULL
                   DO NOTHING"#,
            )
            .bind(Uuid::new_v4()).bind(company_id).bind(run_id)
            .bind(r.invoice_ref).bind(&r.invoice_kind).bind(r.party_id)
            .bind(level).bind(action_type).bind(dpd as i32).bind(r.outstanding_amount)
            .execute(&mut *tx).await?.rows_affected();
            if inserted > 0 { emitted += 1; }
        }

        sqlx::query("UPDATE payment.dunning_runs SET actions_emitted = $2 WHERE id = $1")
            .bind(run_id).bind(emitted).execute(&mut *tx).await?;

        tx.commit().await?;
        Ok((run_id, emitted))
    }
}

// --- helpers ----------------------------------------------------------------

fn bucket_index(dpd: i64) -> usize {
    match dpd {
        d if d <= 0 => 0,   // current
        1..=30 => 1,
        31..=60 => 2,
        61..=90 => 3,
        _ => 4,             // 90+
    }
}

fn bucket_name(dpd: i64) -> &'static str {
    match dpd {
        d if d <= 0 => "current",
        1..=30 => "bucket_1_30",
        31..=60 => "bucket_31_60",
        61..=90 => "bucket_61_90",
        _ => "bucket_90p",
    }
}

/// Decide the dunning level + action type from days-past-due.
fn dunning_level(dpd: i64) -> (&'static str, &'static str) {
    match dpd {
        1..=7 => ("reminder", "send_reminder"),
        8..=30 => ("overdue", "send_overdue"),
        31..=60 => ("final_notice", "send_final_notice"),
        61..=90 => ("collection", "escalate_to_agency"),
        _ => ("written_off", "recommend_write_off"),
    }
}
