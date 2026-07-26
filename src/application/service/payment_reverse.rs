//! Reversing a posted payment — the refund path (hand-authored, user-owned).
//!
//! An `impl PaymentWriteService` chunk over the vocabulary in [`super::payment_write_service`]:
//! build the sign-flipped mirror of the settlement post (`posting_type = "reversal"`), drive the GL
//! sink, flip the entry posted→cancelled, and emit `PaymentCancelled` carrying the allocations so an
//! ACL routes each → `billing::reverse_settlement` to restore the invoices' outstanding.
//!
//! Per the module's 4-layer rule this file holds no SQL — the statements live on
//! `PaymentEntryRepository` / `PaymentAllocationRepository`. The reverse-seam emit is gated on the
//! `posted→cancelled` transition (exactly-once); accounting dedups the reversal post itself on
//! `(company, source_type, source_id, posting_type)`.

use backbone_orm::company_scope;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::payment_events::{PaymentCancelled, PaymentEvent, SettledInvoice};
use super::payment_gl::{AccountingPostEnvelope, GlPostLine, GlPostSink};
use super::payment_write_service::{PaymentError, PaymentWriteService, SettleOutcome};

impl PaymentWriteService {
    /// Build the REVERSAL post — the sign-flipped mirror of the settlement post, `posting_type =
    /// "reversal"`, linked to the original via `reverses_post_id`. Accounting keys idempotency on
    /// `(company, source_type, source_id, posting_type)`, so a reversal (same `source_id`, distinct
    /// `posting_type`) is a separate post from the original AND a re-reversal dedups to one.
    pub async fn build_reversal_post(&self, payment_id: Uuid) -> Result<AccountingPostEnvelope, PaymentError> {
        let orig = self.build_settlement_post(payment_id).await?;
        let reverses_post_id: Option<Uuid> = self.entries.fetch_accounting_post_id(&self.db_pool, payment_id).await?;
        let lines = orig.lines.iter().map(|l| GlPostLine {
            account_id: l.account_id, debit: l.credit, credit: l.debit,
            party_type: l.party_type.clone(), party_id: l.party_id,
            description: l.description.as_ref().map(|d| format!("Reversal: {d}")),
        }).collect();
        let env = AccountingPostEnvelope {
            idempotency_key: format!("reversal:{payment_id}"), posting_type: "reversal".into(),
            reverses_post_id, lines,
            description: orig.description.map(|d| format!("Reversal: {d}")),
            ..orig
        };
        if !env.is_balanced() { return Err(PaymentError::UnbalancedPost); }
        Ok(env)
    }

    /// Reverse a posted payment in full — the refund path (brief KEEP; council 2026-07-05). Posts the
    /// sign-flipped mirror journal (`posting_type="reversal"`) into the ledger and emits
    /// `PaymentCancelled` carrying the allocations, so an ACL routes each → `billing::reverse_settlement`
    /// to restore the invoices' outstanding. **All-or-nothing** (settled allocations AND the on-account
    /// remainder unwind together — a partial reverse would reopen the split invariant). Exactly-once:
    /// accounting dedups the reversal post, and the emit is gated on the `posted→cancelled` transition,
    /// so a repeat call posts + restores once. The exit for an on-account credit or a wrong settlement —
    /// the operator never hand-edits posted GL.
    pub async fn reverse_payment(&self, payment_id: Uuid, sink: &dyn GlPostSink) -> Result<SettleOutcome, PaymentError> {
        // RLS scope (ADR-0008), ID-only: fenced by the request/inherited scope.
        let status: String = self.entries.fetch_status(&self.db_pool, payment_id).await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if status != "posted" && status != "cancelled" {
            return Err(PaymentError::NotReversible(status));
        }
        let env = self.build_reversal_post(payment_id).await?;
        match sink.post(&env).await {
            Ok(ack) => {
                let rows_affected = company_scope::with_company_scope(
                    Some(env.company_id),
                    self.entries.mark_cancelled(&self.db_pool, payment_id),
                ).await?;
                // Only the invocation that flipped posted→cancelled emits — so the reverse-seam restores
                // each invoice exactly once even under a repeat/concurrent reverse.
                if rows_affected == 1 {
                    self.emit_cancelled(payment_id, &env, &ack).await?;
                }
                Ok(SettleOutcome {
                    payment_id, post_id: ack.post_id, journal_id: ack.journal_id,
                    idempotent_reuse: ack.idempotent_reuse || rows_affected == 0,
                })
            }
            Err(rej) => Err(PaymentError::GlRejected { code: rej.code, message: rej.message }),
        }
    }

    async fn emit_cancelled(&self, payment_id: Uuid, env: &AccountingPostEnvelope, ack: &super::payment_gl::GlPostAck) -> Result<(), PaymentError> {
        let hdr = company_scope::with_company_scope(
            Some(env.company_id),
            self.entries.fetch_type_and_amount(&self.db_pool, payment_id),
        ).await?;
        let payment_type: String = hdr.payment_type;
        let paid_amount: Decimal = hdr.paid_amount;
        let alloc_rows = company_scope::with_company_scope(
            Some(env.company_id),
            self.allocations.fetch_for_payment(&self.db_pool, payment_id),
        ).await?;
        let allocations: Vec<SettledInvoice> = alloc_rows.into_iter().map(|r| SettledInvoice {
            invoice_ref: r.invoice_ref, invoice_kind: r.invoice_kind, amount: r.allocated_amount,
        }).collect();
        self.sink.publish(PaymentEvent::PaymentCancelled(PaymentCancelled {
            payment_id, company_id: env.company_id, journal_id: ack.journal_id, post_id: ack.post_id,
            payment_type, allocations, paid_amount,
            correlation_id: None, causation_id: None,
        }));
        Ok(())
    }
}
