//! Payment domain events (hand-authored, user-owned) — the public extension surface.
//!
//! `PaymentSettled` carries the per-invoice knock-offs so an ACL routes them to
//! `backbone-billing::apply_settlement` — drawing down each invoice's `outstanding_amount` and
//! payment schedules, and flipping its status to `partially_paid`/`paid`. This is the seam that
//! closes the cash loop (order-to-cash-to-bank, procure-to-pay-to-bank).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One invoice knock-off carried by `PaymentSettled` — how much of `invoice_ref` this payment paid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettledInvoice {
    pub invoice_ref: Uuid,
    /// "sales" | "purchase" — which billing invoice table `invoice_ref` points at.
    pub invoice_kind: String,
    pub amount: Decimal,
}

/// A payment posted to the GL and knocked off its allocated invoices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentSettled {
    pub payment_id: Uuid,
    pub company_id: Uuid,
    pub journal_id: Uuid,
    pub post_id: Uuid,
    /// "receive" | "pay".
    pub payment_type: String,
    pub allocations: Vec<SettledInvoice>,
    pub paid_amount: Decimal,
}

/// A payment received on account (no invoice allocation) — an unlinked credit awaiting reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentReceivedOnAccount {
    pub payment_id: Uuid,
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
    pub company_id: Uuid,
    pub journal_id: Uuid,
    pub post_id: Uuid,
    /// "receive" | "pay".
    pub payment_type: String,
    pub allocations: Vec<SettledInvoice>,
    pub paid_amount: Decimal,
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
