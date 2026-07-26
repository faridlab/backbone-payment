//! Creating a payment + its allocations (hand-authored, user-owned).
//!
//! An `impl PaymentWriteService` chunk over the vocabulary in [`super::payment_write_service`]:
//! validate the basket (`paid_amount > 0`, no negative allocation, `Σ allocations ≤ paid_amount`)
//! and persist the entry + its allocations as ONE unit of work.
//!
//! Per the module's 4-layer rule this file holds no SQL — the statements live on
//! `PaymentEntryRepository` / `PaymentAllocationRepository`, whose insert methods take THIS service's
//! transaction so a payment is never half-written.

use backbone_orm::company_scope;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::infrastructure::persistence::{NewAllocationRow, NewPaymentEntryRow};

use super::payment_write_service::{is_dup, money, NewPayment, PaymentError, PaymentWriteService};

impl PaymentWriteService {
    /// Validate + persist a payment and its allocations. **Payment-local invariant:**
    /// `Σ allocations ≤ paid_amount` (you cannot allocate more money than moved). Per-invoice
    /// over-settlement is billing's invariant (`apply_settlement`), not enforced here.
    pub async fn create_payment(&self, p: NewPayment) -> Result<Uuid, PaymentError> {
        if p.paid_amount <= Decimal::ZERO {
            return Err(PaymentError::NonPositiveAmount);
        }
        let mut allocated = Decimal::ZERO;
        for a in &p.allocations {
            if a.amount < Decimal::ZERO { return Err(PaymentError::NegativeAmount); }
            allocated += a.amount;
        }
        let paid = money(p.paid_amount);
        let allocated = money(allocated);
        if allocated > paid {
            return Err(PaymentError::OverAllocated { paid, allocated });
        }
        let unallocated = paid - allocated;
        let id = Uuid::new_v4();
        let currency = p.currency.clone().unwrap_or_else(|| "IDR".into());

        // RLS scope (ADR-0008): company on the DTO — bind it onto the transaction so the entry +
        // allocations insert fenced (WITH CHECK sees the caller's company).
        let mut tx = self.db_pool.begin().await?;
        company_scope::bind_company_on(&mut tx, p.company_id).await?;
        let r = self.entries.insert_entry(&mut tx, &NewPaymentEntryRow {
            id,
            payment_number: &p.payment_number,
            company_id: p.company_id,
            branch_id: p.branch_id,
            payment_type: &p.payment_type,
            party_type: p.party_type.as_deref(),
            party_id: p.party_id,
            posting_date: p.posting_date,
            currency: &currency,
            mode_of_payment_id: p.mode_of_payment_id,
            paid_amount: paid,
            allocated_amount: allocated,
            unallocated_amount: unallocated,
            bank_account_id: p.bank_account_id,
            party_account_id: p.party_account_id,
            reference_no: p.reference_no.as_deref(),
            withholding_amount: p.withholding_amount,
            withholding_account_id: p.withholding_account_id,
            withholding_tax_type: &p.withholding_tax_type,
        }).await;
        if let Err(e) = r {
            return Err(if is_dup(&e) { PaymentError::DuplicateNumber(p.payment_number) } else { e.into() });
        }
        for a in &p.allocations {
            self.allocations.insert_allocation(&mut tx, &NewAllocationRow {
                id: Uuid::new_v4(),
                company_id: p.company_id,
                payment_id: id,
                invoice_ref: a.invoice_ref,
                invoice_kind: &a.invoice_kind,
                allocated_amount: money(a.amount),
            }).await?;
        }
        tx.commit().await?;
        Ok(id)
    }
}
