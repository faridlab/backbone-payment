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
//! Tenancy (ADR-0029): the module owns no scoping column — the composing service's tenancy decorator
//! does. Write transactions relay the AMBIENT org request scope when the caller bound one; the
//! legacy company carried by that scope is what the GL envelope, the seam events, and the
//! still-company-fenced downstream ports receive.
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

use crate::infrastructure::persistence::{PaymentAllocationRepository, PaymentEntryRepository};

use super::payment_events::{LoggingSink, PaymentEventSink};

pub(super) fn money(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// The legacy company twin (ADR-0029) — the company value downstream statements still need
/// (the GL envelope, the seam events billing consumes, the still-company-fenced port calls).
/// The module stores no tenancy key, so the twin is read from the AMBIENT org request scope
/// the composing service bound; `None` means the caller is outside any scope, and callers
/// that must carry a company fail closed on it rather than guess.
pub(super) fn legacy_company_twin() -> Option<Uuid> {
    backbone_orm::org_scope::current_org_scope()
        .and_then(|s| s.legacy_company_id())
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
    /// Optional organizational branch label (a plain business column, not a tenancy key).
    pub branch_id: Option<Uuid>,
    /// "receive" | "pay".
    pub payment_type: String,
    /// "customer" | "supplier" | "employee".
    pub party_type: Option<String>,
    pub party_id: Option<Uuid>,
    pub posting_date: chrono::NaiveDate,
    pub currency: Option<String>,
    pub mode_of_payment_id: Option<Uuid>,
    /// "manual" | "bank_transfer" | "cash" | "cheque" | "gateway" — the channel dimension that
    /// decides the post's landing state (`None` = manual).
    pub method: Option<String>,
    /// The gateway/provider's transaction, when the channel is a gateway.
    pub provider_txn_id: Option<Uuid>,
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
    OverAllocated {
        paid: Decimal,
        allocated: Decimal,
    },
    UnbalancedPost,
    DuplicateNumber(String),
    PaymentNotFound(Uuid),
    UnknownPaymentType(String),
    UnknownPaymentMethod(String),
    NotReversible(String),
    NotSubmittable(String),
    NotRejectable(String),
    /// The payment's fused status is terminal or already landed — posting refuses BEFORE the GL sink
    /// is driven, so a rejected/cancelled payment can never leave a journal the entry then disowns.
    NotPostable(String),
    ReconcilableProbeRefused(String),
    /// No legacy company twin is reachable for a statement that must carry one (the GL envelope,
    /// the seam events, the still-company-fenced downstream ports). The module stores no tenancy
    /// key (ADR-0029) — the twin comes from the ambient org request scope, so a caller outside
    /// one cannot post.
    TenancyContextMissing,
    GlRejected {
        code: String,
        message: String,
    },
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
            PaymentError::UnknownPaymentMethod(_) => "unknown_payment_method".into(),
            PaymentError::NotReversible(_) => "not_reversible".into(),
            PaymentError::NotSubmittable(_) => "not_submittable".into(),
            PaymentError::NotRejectable(_) => "not_rejectable".into(),
            PaymentError::NotPostable(_) => "not_postable".into(),
            PaymentError::ReconcilableProbeRefused(_) => "reconcilable_probe_refused".into(),
            PaymentError::TenancyContextMissing => "tenancy_context_missing".into(),
            PaymentError::GlRejected { code, .. } => code.clone(),
            PaymentError::Db(_) => "internal_error".into(),
        }
    }
    pub fn http_status(&self) -> u16 {
        match self {
            PaymentError::PaymentNotFound(_) => 404,
            PaymentError::ReconcilableProbeRefused(_) => 500,
            PaymentError::TenancyContextMissing => 500,
            PaymentError::Db(_) => 500,
            _ => 422,
        }
    }
}
impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentError::GlRejected { code, message } => write!(f, "{code}: {message}"),
            PaymentError::OverAllocated { paid, allocated } => {
                write!(f, "over_allocated: allocated {allocated} > paid {paid}")
            }
            PaymentError::NotSubmittable(s) => write!(f, "not_submittable: payment is {s}"),
            PaymentError::NotRejectable(s) => write!(f, "not_rejectable: payment is {s}"),
            PaymentError::NotPostable(s) => write!(f, "not_postable: payment is {s}"),
            PaymentError::ReconcilableProbeRefused(m) => {
                write!(f, "reconcilable_probe_refused: {m}")
            }
            PaymentError::TenancyContextMissing => write!(
                f,
                "tenancy_context_missing: no legacy company twin in scope — the post's GL \
                 envelope and seam events carry the caller's company; run under the composing \
                 service's org request scope"
            ),
            other => write!(f, "{}", other.code()),
        }
    }
}
impl std::error::Error for PaymentError {}
impl From<sqlx::Error> for PaymentError {
    fn from(e: sqlx::Error) -> Self {
        PaymentError::Db(e)
    }
}
/// Discriminate a unique violation out of a raw `sqlx::Error`.
///
/// This is why the repositories' write methods leak `sqlx::Error` rather than a typed repo error: the
/// service turns a re-used payment number into `DuplicateNumber`, and a typed error would have thrown
/// that information away.
pub(super) fn is_dup(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .map(|d| d.is_unique_violation())
        .unwrap_or(false)
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
    /// The early-pay-discount resolver (billing owns the invoice's discount block; payment owns the
    /// taking). Defaults to no-discount; the composing host injects the adapter.
    pub(super) discount: Arc<dyn super::payment_discount::SettlementDiscountPort>,
    /// The bank-account reconcilability read (accounting owns the flag; payment computes the landing
    /// state from it). Defaults to the real `to_regclass`-guarded read; tests inject a stub.
    pub(super) reconcilable: Arc<dyn super::payment_lifecycle::BankReconcilablePort>,
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
            discount: Arc::new(super::payment_discount::NoDiscount),
            reconcilable: Arc::new(super::payment_lifecycle::AccountingReconcilableRead),
        }
    }
    /// Enable crash-safe `PaymentSettled` emission via the durable outbox in `schema` (e.g. `"payment"`).
    /// Requires `backbone_outbox::outbox::migrate` to have created `<schema>.outbox_events`.
    pub fn with_outbox_schema(mut self, schema: impl Into<String>) -> Self {
        self.outbox_schema = Some(schema.into());
        self
    }
    /// Inject the early-pay-discount resolver (the billing adapter at composition time).
    pub fn with_discount_port(
        mut self,
        port: Arc<dyn super::payment_discount::SettlementDiscountPort>,
    ) -> Self {
        self.discount = port;
        self
    }
    /// Inject the bank-reconcilability read (module tests inject a stub; the default reads
    /// accounting through a `to_regclass` guard).
    pub fn with_reconcilable_port(
        mut self,
        port: Arc<dyn super::payment_lifecycle::BankReconcilablePort>,
    ) -> Self {
        self.reconcilable = port;
        self
    }
}
