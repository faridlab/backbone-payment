// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

pub mod payment_events;
pub mod payment_gl;
pub mod payment_write_service;
// The write surface, chunked: each is an `impl PaymentWriteService` block over the vocabulary that
// stays in `payment_write_service` (so the `payment_write_service::{NewPayment, ...}` import paths
// are unchanged).
pub mod billing_receivables_port;
pub mod payment_create;
pub mod payment_discount;
pub mod payment_dunning_service;
pub mod payment_lifecycle;
pub mod payment_reverse;
pub mod payment_settle;
pub use billing_receivables_port::{BillingReceivablesPort, ReceivableRow};
pub use payment_discount::{discount_for, EarlyPayDecision, NoDiscount, SettlementDiscountPort};
pub use payment_dunning_service::{PaymentDunningError, PaymentDunningService};
pub use payment_lifecycle::{landing_state, AccountingReconcilableRead, BankReconcilablePort};
pub use payment_events::{
    LoggingSink, PaymentCancelled, PaymentEvent, PaymentEventSink, PaymentReceivedOnAccount,
    PaymentSettled, SettledInvoice,
};
pub use payment_gl::{AccountingPostEnvelope, GlPostAck, GlPostLine, GlPostRejected, GlPostSink};
pub use payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService, SettleOutcome,
};
