//! Validated write path + settlement posting engine for payment (hand-authored, user-owned).
//!
//! A PaymentEntry records money moving and allocates it across invoices. On post it assembles ONE
//! balanced settlement `AccountingPost`:
//!   - **receive:** `Dr Bank (paid) · Cr A/R (paid) [customer]`
//!   - **pay:**     `Dr A/P (paid) [supplier] · Cr Bank (paid)`
//! then emits `PaymentSettled{allocations}` so an ACL knocks each invoice down in billing.
//!
//! Bounded-context split: THIS module owns "you cannot allocate more money than moved"
//! (`Σ allocations ≤ paid_amount`); billing owns "you cannot knock off more than is owed"
//! (`amount ≤ outstanding`, enforced in `apply_settlement`). Posting is idempotent (source_id =
//! payment id); the seam event is gated on the pending→posted transition, never re-emitted on a
//! concurrent double-post (the lesson from billing's council).
//!
//! **Layering (the module's 4-layer rule):** this service ORCHESTRATES — it validates, computes the
//! money, owns the unit of work (`begin`/`commit`), builds the GL envelope, drives the sink, and
//! publishes events. It holds no SQL: every statement lives on `PaymentEntryRepository` /
//! `PaymentAllocationRepository`, whose custom methods take the caller's transaction so a cross-entity
//! write (the entry + its allocations; the posted-transition + the outbox stage) commits as one unit.
//! The RLS scope wrappers (ADR-0008) stay HERE, in the service, because the service is what knows the
//! company; tx-taking repo methods ride the bind this service already made.
//!
//! **This file is the hub:** it holds the module's vocabulary (input structs, outcomes, errors) and
//! the service constructor. The rest of the write surface is chunked into focused siblings, each an
//! `impl PaymentWriteService` block over these same types:
//!
//! - [`super::payment_create`] — validate + persist a payment and its allocations (`create_payment`).
//! - [`super::payment_settle`] — assemble + post the settlement journal and emit `PaymentSettled`
//!   (`build_settlement_post`, `post_payment`, the durable outbox stage, the seam emit, and the
//!   posted-short-circuit).
//! - [`super::payment_reverse`] — refund / bounced cheque / mis-applied (`build_reversal_post`,
//!   `reverse_payment`, emit `PaymentCancelled`).

use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::infrastructure::persistence::{
    PaymentAllocationRepository, PaymentEntryRepository,
};

use super::payment_events::{PaymentEventSink, LoggingSink};

pub(super) fn money(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

// --- input structs -----------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NewAllocation {
    pub invoice_ref: Uuid,
    /// "sales" | "purchase".
    pub invoice_kind: String,
    pub amount: Decimal,
}

#[derive(Debug, Clone)]
pub struct NewPayment {
    pub payment_number: String,
    pub company_id: Uuid,
    pub branch_id: Option<Uuid>,
    /// "receive" | "pay".
    pub payment_type: String,
    /// "customer" | "supplier" | "employee".
    pub party_type: Option<String>,
    pub party_id: Option<Uuid>,
    pub posting_date: chrono::NaiveDate,
    pub currency: Option<String>,
    pub mode_of_payment_id: Option<Uuid>,
    pub bank_account_id: Uuid,
    pub party_account_id: Uuid,
    pub paid_amount: Decimal,
    pub reference_no: Option<String>,
    pub allocations: Vec<NewAllocation>,
    /// PPh (withholding tax) — ADR-003. 0 = no withholding (2-line post). > 0 adds a third line.
    pub withholding_amount: Decimal,
    pub withholding_account_id: Option<Uuid>,
    /// "none" | "pph_22" | "pph_23" | "pph_26".
    pub withholding_tax_type: String,
}

#[derive(Debug, Clone)]
pub struct SettleOutcome {
    pub payment_id: Uuid,
    pub post_id: Uuid,
    pub journal_id: Uuid,
    pub idempotent_reuse: bool,
}

// --- errors ------------------------------------------------------------------

#[derive(Debug)]
pub enum PaymentError {
    NonPositiveAmount,
    NegativeAmount,
    UnsupportedCurrency(String),
    OverAllocated { paid: Decimal, allocated: Decimal },
    UnbalancedPost,
    DuplicateNumber(String),
    PaymentNotFound(Uuid),
    UnknownPaymentType(String),
    NotReversible(String),
    GlRejected { code: String, message: String },
    Db(sqlx::Error),
}

impl PaymentError {
    pub fn code(&self) -> String {
        match self {
            PaymentError::NonPositiveAmount => "non_positive_amount".into(),
            PaymentError::NegativeAmount => "negative_amount".into(),
            PaymentError::UnsupportedCurrency(_) => "unsupported_currency".into(),
            PaymentError::OverAllocated { .. } => "over_allocated".into(),
            PaymentError::UnbalancedPost => "unbalanced_post".into(),
            PaymentError::DuplicateNumber(_) => "duplicate_number".into(),
            PaymentError::PaymentNotFound(_) => "payment_not_found".into(),
            PaymentError::UnknownPaymentType(_) => "unknown_payment_type".into(),
            PaymentError::NotReversible(_) => "not_reversible".into(),
            PaymentError::GlRejected { code, .. } => code.clone(),
            PaymentError::Db(_) => "internal_error".into(),
        }
    }
    pub fn http_status(&self) -> u16 {
        match self {
            PaymentError::PaymentNotFound(_) => 404,
            PaymentError::Db(_) => 500,
            _ => 422,
        }
    }
}
impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentError::GlRejected { code, message } => write!(f, "{code}: {message}"),
            PaymentError::OverAllocated { paid, allocated } => write!(f, "over_allocated: allocated {allocated} > paid {paid}"),
            other => write!(f, "{}", other.code()),
        }
    }
}
impl std::error::Error for PaymentError {}
impl From<sqlx::Error> for PaymentError {
    fn from(e: sqlx::Error) -> Self { PaymentError::Db(e) }
}
/// Discriminate a unique violation out of a raw `sqlx::Error`.
///
/// This is why the repositories' write methods leak `sqlx::Error` rather than a typed repo error: the
/// service turns a re-used payment number into `DuplicateNumber`, and a typed error would have thrown
/// that information away.
pub(super) fn is_dup(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false)
}

/// The repositories are held behind `Arc` only so this service stays `Clone` —
/// `GenericCrudRepository` is not itself `Clone`. They are stateless handles over the same pool.
#[derive(Clone)]
pub struct PaymentWriteService {
    pub(super) db_pool: PgPool,
    pub(super) sink: Arc<dyn PaymentEventSink>,
    pub(super) entries: Arc<PaymentEntryRepository>,
    pub(super) allocations: Arc<PaymentAllocationRepository>,
    /// When set, `post_payment` stages `PaymentSettled` into `<schema>.outbox_events` **inside the
    /// posted-transition transaction** (crash-safe emission — go-live durable bus). When `None`, only the
    /// legacy in-proc sink fires (existing behaviour). The relay drains the outbox to the real bus.
    pub(super) outbox_schema: Option<String>,
}

impl PaymentWriteService {
    pub fn new(db_pool: PgPool) -> Self {
        Self::with_sink(db_pool, Arc::new(LoggingSink))
    }
    pub fn with_sink(db_pool: PgPool, sink: Arc<dyn PaymentEventSink>) -> Self {
        Self {
            entries: Arc::new(PaymentEntryRepository::new(db_pool.clone())),
            allocations: Arc::new(PaymentAllocationRepository::new(db_pool.clone())),
            db_pool,
            sink,
            outbox_schema: None,
        }
    }
    /// Enable crash-safe `PaymentSettled` emission via the durable outbox in `schema` (e.g. `"payment"`).
    /// Requires `backbone_outbox::outbox::migrate` to have created `<schema>.outbox_events`.
    pub fn with_outbox_schema(mut self, schema: impl Into<String>) -> Self {
        self.outbox_schema = Some(schema.into());
        self
    }
}
