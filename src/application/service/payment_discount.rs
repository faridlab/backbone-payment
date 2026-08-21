//! The early-pay-discount settlement port (hand-authored, user-owned).
//!
//! Payment owns WHEN a discount decision is needed (at post, per allocation) and WHAT it does to the
//! money (the third GL leg + the stamped allocation row); the invoice-owning module owns WHETHER one
//! applies — that is a read of the invoice's materialized early-pay block (window, percent, expense
//! account), which lives in billing. This port is the only meeting place: the composing host injects
//! an adapter over billing's `resolve_early_pay_discount`, so payment carries no Cargo edge into
//! billing and billing knows nothing of the payment lifecycle.
//!
//! The decision carries PERCENT + account + the invoice's outstanding BASIS, not an amount: the
//! discount is taken on what THIS payment allocates against that invoice (a partial payment earns
//! the discount on the partial amount), clamped to the basis — payment is the only party that
//! knows the allocated amount at decision time, and the basis is what bounds the cumulative take:
//! one payment may allocate more than the invoice asks, but the discount may never exceed
//! `percent × outstanding-at-resolve`.

use rust_decimal::Decimal;
use uuid::Uuid;

use super::payment_write_service::PaymentError;

/// An applicable early-pay discount: the percent to take, the account the discount lands on, and
/// the invoice's outstanding at resolve time (the cumulative bound). `account_id` is non-optional
/// by contract — a discount with nowhere to land is the invoicing module's validation failure,
/// never payment's fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct EarlyPayDecision {
    pub percent: Decimal,
    pub outstanding_basis: Decimal,
    pub account_id: Uuid,
}

/// Resolve whether an early-pay discount applies to one invoice on a given date. `Ok(None)` is a
/// legitimate answer ("no discount") and never an error; errors mean the probe itself failed.
#[async_trait::async_trait]
pub trait SettlementDiscountPort: Send + Sync {
    async fn resolve(
        &self,
        company_id: Uuid,
        invoice_ref: Uuid,
        invoice_kind: &str,
        on_date: chrono::NaiveDate,
    ) -> Result<Option<EarlyPayDecision>, PaymentError>;
}

/// The no-discount default: payments post with no discount leg until a host injects the real
/// adapter. Keeps module-default wiring (and the module's own tests) free of any invoice-module
/// dependency.
pub struct NoDiscount;

#[async_trait::async_trait]
impl SettlementDiscountPort for NoDiscount {
    async fn resolve(
        &self,
        _company_id: Uuid,
        _invoice_ref: Uuid,
        _invoice_kind: &str,
        _on_date: chrono::NaiveDate,
    ) -> Result<Option<EarlyPayDecision>, PaymentError> {
        Ok(None)
    }
}

/// Compute one allocation's discount from a decision: `money(allocated × percent / 100)`, clamped to
/// the allocated amount (a discount can never exceed what moved) and to zero (a non-positive
/// resolution takes nothing). Pure — the golden tests pin it.
pub fn discount_for(allocated: Decimal, decision: &EarlyPayDecision) -> Decimal {
    if allocated <= Decimal::ZERO || decision.percent <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let raw = allocated * decision.percent / Decimal::from(100u32);
    let rounded = super::payment_write_service::money(raw);
    if rounded > allocated {
        allocated
    } else {
        rounded
    }
}
