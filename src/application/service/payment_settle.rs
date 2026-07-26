//! Posting a payment — assemble + commit the settlement journal (hand-authored, user-owned).
//!
//! An `impl PaymentWriteService` chunk over the vocabulary in [`super::payment_write_service`]:
//! build the balanced settlement `AccountingPostEnvelope` (receive: `Dr Bank · [Dr PPh Receivable] ·
//! Cr A/R`; pay: `Dr A/P · Cr Bank · [Cr PPh Payable]`), drive the GL sink, flip the entry
//! pending→posted, and emit `PaymentSettled` (plus `PaymentReceivedOnAccount` when there is an
//! unallocated remainder). Posting is idempotent (source_id = payment id); the seam event is gated on
//! the pending→posted transition — only the invocation that flips the state publishes, so a concurrent
//! double-post can never draw an invoice's outstanding down twice.
//!
//! Per the module's 4-layer rule this file holds no SQL — the statements live on
//! `PaymentEntryRepository` / `PaymentAllocationRepository`. The posted-transition AND the durable
//! outbox stage ride THIS service's transaction so a crash after the transition can never lose the
//! `PaymentSettled` event (go-live durable bus).

use backbone_orm::company_scope;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::payment_events::{
    PaymentEvent, PaymentReceivedOnAccount, PaymentSettled, SettledInvoice,
};
use super::payment_gl::{AccountingPostEnvelope, GlPostLine, GlPostSink};
use super::payment_write_service::{PaymentError, PaymentWriteService, SettleOutcome};

impl PaymentWriteService {
    /// Build the balanced settlement post. receive: `Dr Bank (net) · [Dr PPh Receivable] · Cr A/R (gross)`;
    /// pay: `Dr A/P (gross) · Cr Bank (net) · [Cr PPh Payable]`. The A/R/A/P control is settled at the
    /// GROSS paid_amount; the bank moves the NET (gross − withholding). When withholding = 0 the post
    /// is the original 2-line shape (backward-compatible). ADR-003.
    pub async fn build_settlement_post(&self, payment_id: Uuid) -> Result<AccountingPostEnvelope, PaymentError> {
        // RLS scope (ADR-0008), ID-only: fenced by the request/inherited scope.
        let p = self.entries.fetch_post_source(&self.db_pool, payment_id).await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        let currency = p.currency.clone();
        if currency != "IDR" { return Err(PaymentError::UnsupportedCurrency(currency)); }
        let payment_type = p.payment_type.clone();
        let paid: Decimal = p.paid_amount;
        let number: String = p.payment_number.clone();
        let bank: Uuid = p.bank_account_id;
        let control: Uuid = p.party_account_id;
        let party_id: Option<Uuid> = p.party_id;
        let party_type: Option<String> = p.party_type.clone();

        let lines = match payment_type.as_str() {
            "receive" => {
                // Dr Bank (net) · [Dr PPh Receivable (withheld)] · Cr A/R (gross)
                let mut ar = GlPostLine::credit(control, paid).with_description(format!("A/R settled {number}"));
                if let (Some(pt), Some(pid)) = (party_type.as_deref(), party_id) { ar = ar.with_party(pt, pid); }
                let mut lines = vec![GlPostLine::debit(bank, paid - p.withholding_amount).with_description(format!("Receipt {number}"))];
                if p.withholding_amount > Decimal::ZERO {
                    let wht = p.withholding_account_id.ok_or(PaymentError::OverAllocated { paid: Decimal::ZERO, allocated: Decimal::ZERO })?;
                    lines.push(GlPostLine::debit(wht, p.withholding_amount).with_description(format!("PPh withheld {number}")));
                }
                lines.push(ar);
                lines
            }
            "pay" => {
                // Dr A/P (gross) · Cr Bank (net) · [Cr PPh Payable (withheld)]
                let mut ap = GlPostLine::debit(control, paid).with_description(format!("A/P settled {number}"));
                if let (Some(pt), Some(pid)) = (party_type.as_deref(), party_id) { ap = ap.with_party(pt, pid); }
                let mut lines = vec![ap];
                lines.push(GlPostLine::credit(bank, paid - p.withholding_amount).with_description(format!("Payment {number}")));
                if p.withholding_amount > Decimal::ZERO {
                    let wht = p.withholding_account_id.ok_or(PaymentError::OverAllocated { paid: Decimal::ZERO, allocated: Decimal::ZERO })?;
                    lines.push(GlPostLine::credit(wht, p.withholding_amount).with_description(format!("PPh withheld {number}")));
                }
                lines
            }
            other => return Err(PaymentError::UnknownPaymentType(other.to_string())),
        };

        let env = AccountingPostEnvelope {
            idempotency_key: payment_id.to_string(), company_id: p.company_id, branch_id: p.branch_id,
            source_type: "payment".into(), source_id: payment_id, source_reference: Some(number),
            posting_date: p.posting_date, currency, posting_type: "original".into(), reverses_post_id: None,
            description: Some(format!("Payment ({payment_type})")), lines,
        };
        if !env.is_balanced() { return Err(PaymentError::UnbalancedPost); }
        Ok(env)
    }

    pub async fn post_payment(&self, payment_id: Uuid, sink: &dyn GlPostSink) -> Result<SettleOutcome, PaymentError> {
        if let Some(o) = self.short_circuit_posted(payment_id).await? { return Ok(o); }
        let env = self.build_settlement_post(payment_id).await?;
        match sink.post(&env).await {
            Ok(ack) => {
                // Gate the reconcile + seam event on THIS invocation performing the pending→posted
                // transition — the seam routes `PaymentSettled` into billing::apply_settlement, so a
                // double-emit would draw an invoice's outstanding down twice. Only the winner publishes.
                // The transition AND the durable outbox stage commit in ONE tx, so a crash after the
                // transition can never lose the `PaymentSettled` event (go-live durable bus).
                let mut tx = self.db_pool.begin().await?;
                company_scope::bind_company_on(&mut tx, env.company_id).await?;
                let rows_affected = self.entries
                    .mark_posted(&mut tx, payment_id, ack.journal_id, ack.post_id).await?;
                if rows_affected == 0 {
                    tx.rollback().await?;
                    return self.short_circuit_posted(payment_id).await?
                        .ok_or(PaymentError::PaymentNotFound(payment_id));
                }
                if let Some(schema) = self.outbox_schema.clone() {
                    self.stage_settled(&mut tx, &schema, payment_id, &env, &ack).await?;
                }
                tx.commit().await?;
                self.emit_settled(payment_id, &env, &ack).await?;
                Ok(SettleOutcome { payment_id, post_id: ack.post_id, journal_id: ack.journal_id, idempotent_reuse: ack.idempotent_reuse })
            }
            Err(rej) => {
                // Deliberately ignored: the GL rejection below is the error being reported, and a
                // failure to mark the state must not mask it.
                let _ = self.entries.mark_failed(&self.db_pool, payment_id).await;
                Err(PaymentError::GlRejected { code: rej.code, message: rej.message })
            }
        }
    }

    /// Stage `PaymentSettled` (with all its allocations) into the durable outbox, reading the payment +
    /// allocations on the SAME transaction as the posted-transition so the event is atomic with the
    /// state change. The relay later delivers it; billing's `apply_settlements_once` dedups it.
    async fn stage_settled(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        schema: &str,
        payment_id: Uuid,
        env: &AccountingPostEnvelope,
        ack: &super::payment_gl::GlPostAck,
    ) -> Result<(), PaymentError> {
        let hdr = self.entries.fetch_type_and_amount_on(&mut **tx, payment_id).await?;
        let payment_type: String = hdr.payment_type;
        let paid_amount: Decimal = hdr.paid_amount;
        let alloc_rows = self.allocations.fetch_for_payment_on(&mut **tx, payment_id).await?;
        let allocations: Vec<serde_json::Value> = alloc_rows.iter().map(|r| serde_json::json!({
            "invoice_ref": r.invoice_ref.to_string(),
            "invoice_kind": r.invoice_kind,
            "amount": r.allocated_amount.to_string(),
        })).collect();
        let payload = serde_json::json!({
            "payment_id": payment_id.to_string(),
            "company_id": env.company_id.to_string(),
            "payment_type": payment_type,
            "paid_amount": paid_amount.to_string(),
            "journal_id": ack.journal_id.to_string(),
            "post_id": ack.post_id.to_string(),
            "allocations": allocations,
        });
        let rec = backbone_outbox::OutboxRecord::new(
            "PaymentSettled", "Payment", payment_id.to_string(), env.company_id, payload, chrono::Utc::now());
        backbone_outbox::outbox::stage(&mut **tx, schema, &rec)
            .await
            .map_err(|e| PaymentError::Db(sqlx::Error::Protocol(e.to_string())))?;
        Ok(())
    }

    async fn emit_settled(&self, payment_id: Uuid, env: &AccountingPostEnvelope, ack: &super::payment_gl::GlPostAck) -> Result<(), PaymentError> {
        let hdr = company_scope::with_company_scope(
            Some(env.company_id),
            self.entries.fetch_settled_header(&self.db_pool, payment_id),
        ).await?;
        let payment_type: String = hdr.payment_type;
        let paid_amount: Decimal = hdr.paid_amount;
        let unallocated: Decimal = hdr.unallocated_amount;
        let party_id: Option<Uuid> = hdr.party_id;

        let alloc_rows = company_scope::with_company_scope(
            Some(env.company_id),
            self.allocations.fetch_for_payment(&self.db_pool, payment_id),
        ).await?;
        let allocations: Vec<SettledInvoice> = alloc_rows.into_iter().map(|r| SettledInvoice {
            invoice_ref: r.invoice_ref, invoice_kind: r.invoice_kind, amount: r.allocated_amount,
        }).collect();

        self.sink.publish(PaymentEvent::PaymentSettled(PaymentSettled {
            payment_id, company_id: env.company_id, journal_id: ack.journal_id, post_id: ack.post_id,
            payment_type, allocations, paid_amount,
            correlation_id: None, causation_id: None,
        }));
        if unallocated > Decimal::ZERO {
            self.sink.publish(PaymentEvent::PaymentReceivedOnAccount(PaymentReceivedOnAccount {
                payment_id, company_id: env.company_id, party_id, unallocated_amount: unallocated,
            }));
        }
        Ok(())
    }

    async fn short_circuit_posted(&self, payment_id: Uuid) -> Result<Option<SettleOutcome>, PaymentError> {
        let row = self.entries.fetch_posted_state(&self.db_pool, payment_id).await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if row.posting_state == "posted" {
            if let (Some(j), Some(p)) = (row.journal_id, row.accounting_post_id) {
                return Ok(Some(SettleOutcome { payment_id, post_id: p, journal_id: j, idempotent_reuse: true }));
            }
        }
        Ok(None)
    }
}
