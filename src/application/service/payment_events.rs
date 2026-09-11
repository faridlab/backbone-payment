//! Payment domain events (hand-authored, user-owned) — the public extension surface.
//!
//! `PaymentSettled` carries the per-invoice knock-offs so an ACL routes them to
//! `backbone-billing::apply_settlement` — drawing down each invoice's `outstanding_amount` and
//! payment schedules, and flipping its status to `partially_paid`/`paid`. This is the seam that
//! closes the cash loop (order-to-cash-to-bank, procure-to-pay-to-bank).
//!
//! Tenancy (ADR-0029): payment's own tables carry no tenant column — the composing service's
//! tenancy decorator owns org scoping. The events' `company_id` fields are the documented legacy
//! company twin: they carry the caller's company to consumers whose tables are still
//! company-fenced (billing first among them), resolved from the ambient org request scope at
//! emit time and absent-meaning-refuse for statements that must carry one.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One invoice knock-off carried by `PaymentSettled` — how much of `invoice_ref` this payment paid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettledInvoice {
    pub invoice_ref: Uuid,
    /// "sales" | "purchase" — which billing invoice table `invoice_ref` points at.
    pub invoice_kind: String,
    /// The GROSS knock-off — the invoice's outstanding draws down by this amount regardless of any
    /// discount taken alongside it.
    pub amount: Decimal,
    /// Early-pay discount taken on this allocation (contract v1.1, additive: absent = none). The
    /// materialized decision, so a replayed or mirrored event carries the same numbers the post
    /// committed — the discount window is never re-evaluated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount_amount: Option<Decimal>,
}

/// A payment posted to the GL and knocked off its allocated invoices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentSettled {
    pub payment_id: Uuid,
    /// The legacy company twin (ADR-0029) for still-company-fenced consumers; payment stores no
    /// tenancy key of its own — this resolves from the ambient org request scope at emit time.
    pub company_id: Uuid,
    pub journal_id: Uuid,
    pub post_id: Uuid,
    /// "receive" | "pay".
    pub payment_type: String,
    pub allocations: Vec<SettledInvoice>,
    pub paid_amount: Decimal,
    /// The fused lifecycle status the post landed on (contract v1.1, additive: absent = a
    /// pre-fusion event, treat as posted-equivalent). "in_flight" | "paid" for fresh posts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Correlation id for tracing the settlement flow across modules (gateway → payment → billing → accounting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Causation id — the parent event that caused this one (e.g. the GatewayTransactionSettled id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
}

/// A payment received on account (no invoice allocation) — an unlinked credit awaiting reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentReceivedOnAccount {
    pub payment_id: Uuid,
    /// The legacy company twin (ADR-0029) — see `PaymentSettled`.
    pub company_id: Uuid,
    pub party_id: Option<Uuid>,
    pub unallocated_amount: Decimal,
}

/// A posted payment was reversed (refund / bounced cheque / mis-applied). Carries the reversal GL
/// post + the allocations that were undone, so an ACL routes each → `billing::reverse_settlement`,
/// restoring the invoices' `outstanding_amount` + schedules — the exit for an on-account credit or a
/// wrong settlement. The mirror of `PaymentSettled`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentCancelled {
    pub payment_id: Uuid,
    /// The legacy company twin (ADR-0029) — see `PaymentSettled`.
    pub company_id: Uuid,
    pub journal_id: Uuid,
    pub post_id: Uuid,
    /// "receive" | "pay".
    pub payment_type: String,
    pub allocations: Vec<SettledInvoice>,
    pub paid_amount: Decimal,
    /// Correlation id for tracing the reversal flow (mirror of PaymentSettled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Causation id — the parent event that caused this reversal (e.g. GatewayTransactionRefunded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
}

/// The payment domain-event union.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum PaymentEvent {
    PaymentSettled(PaymentSettled),
    PaymentReceivedOnAccount(PaymentReceivedOnAccount),
    PaymentCancelled(PaymentCancelled),
}

/// Sink for payment domain events. Fire-and-forget; a real adapter wires a bus, tests record.
pub trait PaymentEventSink: Send + Sync {
    fn publish(&self, event: PaymentEvent);
}

/// Default sink — emits structured tracing events.
pub struct LoggingSink;

impl PaymentEventSink for LoggingSink {
    fn publish(&self, event: PaymentEvent) {
        tracing::info!(target: "payment.events", ?event, "payment domain event");
    }
}
