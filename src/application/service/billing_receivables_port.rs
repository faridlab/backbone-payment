//! Billing receivables READ port (hand-authored, user-owned) — the zero-cargo-edge seam for
//! aging/dunning. Payment declares the port; the composition layer implements it by calling
//! billing's repository. The shipped payment library never imports billing.
//!
//! Mirrors the port-trait pattern (GlPostSink, BillingEventSink). See ADR-006.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

/// One outstanding receivable/payable row, as payment's dunning engine sees it.
#[derive(Debug, Clone)]
pub struct ReceivableRow {
    pub invoice_ref: Uuid,
    pub invoice_kind: String,        // "sales" | "purchase"
    pub party_id: Option<Uuid>,
    pub due_date: Option<NaiveDate>, // falls back to posting_date when NULL
    pub outstanding_amount: Decimal,
}

/// Payment's view of billing's outstanding receivables — the aging/dunning READ port.
/// Implemented in the composition layer by an ACL that calls billing's repository.
#[async_trait::async_trait]
pub trait BillingReceivablesPort: Send + Sync {
    /// Every live (non-deleted, posted) invoice of `kind` with non-zero outstanding,
    /// scoped to `company_id`.
    async fn outstanding_for(
        &self,
        company_id: Uuid,
        kind: &str,
        as_of: NaiveDate,
    ) -> Result<Vec<ReceivableRow>, String>;
}
