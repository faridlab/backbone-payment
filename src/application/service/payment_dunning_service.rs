//! The dunning/aging engine (hand-authored, user-owned) — the receivables-timeline read-model +
//! escalation state. Reads billing's outstanding via `BillingReceivablesPort` (zero cargo edge),
//! buckets by days-past-due, and emits dunning actions per invoice.
//!
//! Real-world rule: an invoice ages from its `due_date`; a payment allocation de-ages it.
//!
//! **Layering (the module's 4-layer rule):** this service ORCHESTRATES — it reads the receivables
//! port, computes the days-past-due bucketing + escalation level, owns the unit of work
//! (`begin`/`commit`), and decides what to emit. It holds no SQL: every statement lives on
//! `AgingSnapshotRepository` / `AgingBucketRepository` / `DunningRunRepository` /
//! `DunningActionRepository`, whose custom methods take the caller's transaction so a snapshot +
//! its buckets (and a run + its actions) commit as one unit. The RLS scope wrappers (ADR-0008)
//! stay HERE, in the service, because the service is what knows the company; tx-taking repo
//! methods ride the bind this service already made.

use backbone_orm::company_scope;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::infrastructure::persistence::{
    AgingBucketRepository, AgingSnapshotRepository, AgingTotals, DunningActionRepository,
    DunningRunRepository, NewAgingBucketRow, NewDunningActionRow,
};

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

        let snapshots = AgingSnapshotRepository::new(self.db_pool.clone());
        let buckets = AgingBucketRepository::new(self.db_pool.clone());

        // Idempotent snapshot insert (unique company + as_of + direction).
        let snapshot_id = snapshots
            .upsert_snapshot(&mut *tx, Uuid::new_v4(), company_id, as_of, direction)
            .await?;

        let mut totals = [Decimal::ZERO; 5]; // current, 1_30, 31_60, 61_90, 90p

        for r in &recs {
            let due = r.due_date.unwrap_or(as_of);
            let dpd = as_of.signed_duration_since(due).num_days();
            let idx = bucket_index(dpd);
            totals[idx] += r.outstanding_amount;

            buckets.insert_bucket(&mut *tx, &NewAgingBucketRow {
                id: Uuid::new_v4(),
                snapshot_id,
                company_id,
                invoice_ref: r.invoice_ref,
                invoice_kind: &r.invoice_kind,
                party_id: r.party_id,
                due_date: due,
                days_past_due: dpd as i32,
                outstanding_amount: r.outstanding_amount,
                bucket: bucket_name(dpd),
            }).await?;
        }

        snapshots.update_totals(&mut *tx, snapshot_id, &AgingTotals {
            total_outstanding: totals.iter().copied().sum::<Decimal>(),
            bucket_current: totals[0],
            bucket_1_30: totals[1],
            bucket_31_60: totals[2],
            bucket_61_90: totals[3],
            bucket_90p: totals[4],
        }).await?;

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

        let runs = DunningRunRepository::new(self.db_pool.clone());
        let actions = DunningActionRepository::new(self.db_pool.clone());

        let run_id = runs
            .insert_run(&mut *tx, Uuid::new_v4(), company_id, as_of, direction)
            .await?;

        let mut emitted = 0i32;
        for r in &recs {
            let due = r.due_date.unwrap_or(as_of);
            let dpd = as_of.signed_duration_since(due).num_days();
            if dpd <= 0 { continue; } // not overdue — no action
            let (level, action_type) = dunning_level(dpd);

            let inserted = actions.upsert_action(&mut *tx, &NewDunningActionRow {
                id: Uuid::new_v4(),
                company_id,
                run_id,
                invoice_ref: r.invoice_ref,
                invoice_kind: &r.invoice_kind,
                party_id: r.party_id,
                level,
                action_type,
                days_past_due: dpd as i32,
                outstanding_amount: r.outstanding_amount,
            }).await?;
            if inserted > 0 { emitted += 1; }
        }

        runs.set_actions_emitted(&mut *tx, run_id, emitted).await?;

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
