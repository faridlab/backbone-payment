//! Posting a payment — assemble + commit the settlement journal (hand-authored, user-owned).
//!
//! An `impl PaymentWriteService` chunk over the vocabulary in [`super::payment_write_service`]:
//! build the balanced settlement `AccountingPostEnvelope`, drive the GL sink, land the fused status
//! (in_flight/paid via [`super::payment_lifecycle::landing_state`]), and emit `PaymentSettled`
//! (plus `PaymentReceivedOnAccount` when there is an unallocated remainder).
//!
//! The settlement post with an early-pay discount is THREE-legged per allocation group:
//!   - **receive:** `Dr Bank (net) · [Dr PPh Receivable] · Dr Discount (Σ) · Cr A/R (gross)`
//!   - **pay:**     `Dr A/P (gross) · Cr Bank (net) · [Cr PPh Payable] · Cr Discount (Σ)`
//! The control account settles at the GROSS allocation (billing's knock-off is unchanged); the bank
//! moves NET of the discount; the discount legs land on the accounts the invoice's term named,
//! grouped per distinct account. Balanced by construction: Σ(debits) = paid = Σ(credits).
//!
//! The discount decision is resolved ONCE — at post, through [`super::payment_discount`]'s port —
//! and MATERIALIZED on the allocation rows (amount + account) inside the same transaction as the
//! posted-transition. Replays, the staged/relayed event, and the mirrored reversal all read the
//! stamp; the discount window is never re-evaluated after money moved (R3).
//!
//! Posting is idempotent (source_id = payment id); the seam event is gated on the pending→posted
//! transition — only the invocation that flips the state publishes, so a concurrent double-post can
//! never draw an invoice's outstanding down twice.
//!
//! Per the module's 4-layer rule this file holds no SQL — the statements live on
//! `PaymentEntryRepository` / `PaymentAllocationRepository`. The posted-transition AND the durable
//! outbox stage AND the discount stamps ride THIS service's transaction so a crash after the
//! transition can never lose the `PaymentSettled` event nor split a discount from its journal.

use backbone_orm::company_scope;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use uuid::Uuid;

use super::payment_events::{
    PaymentEvent, PaymentReceivedOnAccount, PaymentSettled, SettledInvoice,
};
use super::payment_gl::{AccountingPostEnvelope, GlPostLine, GlPostSink};
use super::payment_lifecycle::landing_state;
use super::payment_write_service::{PaymentError, PaymentWriteService, SettleOutcome};

use crate::infrastructure::persistence::PostSourceRow;

/// One allocation as the post needs it: the gross knock-off plus the discount decision applied to
/// it (zero when none). `[account, Σ discount]` grouping into GL legs happens at assembly.
pub(super) struct AllocForPost {
    pub invoice_ref: Uuid,
    pub invoice_kind: String,
    pub amount: Decimal,
    pub discount_amount: Decimal,
    pub discount_account_id: Option<Uuid>,
}

impl PaymentWriteService {
    /// Build the balanced settlement post from a fetched source + its allocations (discounts
    /// already decided). Pure assembly — no I/O — so the original post path (decision in memory,
    /// stamps commit after the GL sink) and the reversal path (decision read from the stamps) build
    /// the IDENTICAL envelope for the same committed state.
    fn assemble_settlement_post(
        p: &PostSourceRow,
        payment_id: Uuid,
        allocs: &[AllocForPost],
    ) -> Result<AccountingPostEnvelope, PaymentError> {
        let paid: Decimal = p.paid_amount;
        let number: String = p.payment_number.clone();
        let bank: Uuid = p.bank_account_id;
        let control: Uuid = p.party_account_id;

        // Discounts per named account — distinct accounts stay distinct legs (an A/R discount
        // expense and an A/P discount revenue never merge into one line).
        let mut discounts: BTreeMap<Uuid, Decimal> = BTreeMap::new();
        let mut discount_total = Decimal::ZERO;
        for a in allocs {
            if a.discount_amount > Decimal::ZERO {
                let account = a.discount_account_id.expect(
                    "a stamped discount carries its account; the resolver refuses a decision without one",
                );
                *discounts.entry(account).or_default() += a.discount_amount;
                discount_total += a.discount_amount;
            }
        }
        let bank_move = paid - p.withholding_amount - discount_total;
        if bank_move < Decimal::ZERO {
            return Err(PaymentError::UnbalancedPost);
        }

        let lines = match p.payment_type.as_str() {
            "receive" => {
                // Dr Bank (net) · [Dr PPh Receivable] · Dr Discount (Σ) · Cr A/R (gross)
                let mut ar = GlPostLine::credit(control, paid)
                    .with_description(format!("A/R settled {number}"));
                if let (Some(pt), Some(pid)) = (p.party_type.as_deref(), p.party_id) {
                    ar = ar.with_party(pt, pid);
                }
                let mut lines = vec![GlPostLine::debit(bank, bank_move)
                    .with_description(format!("Receipt {number}"))];
                if p.withholding_amount > Decimal::ZERO {
                    let wht = p
                        .withholding_account_id
                        .ok_or(PaymentError::OverAllocated {
                            paid: Decimal::ZERO,
                            allocated: Decimal::ZERO,
                        })?;
                    lines.push(
                        GlPostLine::debit(wht, p.withholding_amount)
                            .with_description(format!("PPh withheld {number}")),
                    );
                }
                for (account, amount) in &discounts {
                    lines.push(
                        GlPostLine::debit(*account, *amount)
                            .with_description(format!("Early-pay discount {number}")),
                    );
                }
                lines.push(ar);
                lines
            }
            "pay" => {
                // Dr A/P (gross) · Cr Bank (net) · [Cr PPh Payable] · Cr Discount (Σ)
                let mut ap = GlPostLine::debit(control, paid)
                    .with_description(format!("A/P settled {number}"));
                if let (Some(pt), Some(pid)) = (p.party_type.as_deref(), p.party_id) {
                    ap = ap.with_party(pt, pid);
                }
                let mut lines = vec![ap];
                lines.push(
                    GlPostLine::credit(bank, bank_move)
                        .with_description(format!("Payment {number}")),
                );
                if p.withholding_amount > Decimal::ZERO {
                    let wht = p
                        .withholding_account_id
                        .ok_or(PaymentError::OverAllocated {
                            paid: Decimal::ZERO,
                            allocated: Decimal::ZERO,
                        })?;
                    lines.push(
                        GlPostLine::credit(wht, p.withholding_amount)
                            .with_description(format!("PPh withheld {number}")),
                    );
                }
                for (account, amount) in &discounts {
                    lines.push(
                        GlPostLine::credit(*account, *amount)
                            .with_description(format!("Early-pay discount {number}")),
                    );
                }
                lines
            }
            other => return Err(PaymentError::UnknownPaymentType(other.to_string())),
        };

        let env = AccountingPostEnvelope {
            idempotency_key: payment_id.to_string(),
            company_id: p.company_id,
            branch_id: p.branch_id,
            source_type: "payment".into(),
            source_id: payment_id,
            source_reference: Some(number),
            posting_date: p.posting_date,
            currency: p.currency.clone(),
            posting_type: "original".into(),
            reverses_post_id: None,
            description: Some(format!("Payment ({})", p.payment_type)),
            lines,
        };
        if !env.is_balanced() {
            return Err(PaymentError::UnbalancedPost);
        }
        Ok(env)
    }

    /// Build the settlement post from the COMMITTED state — the discounts read from their stamps.
    /// The reversal mirrors this envelope; direct callers get the post the entry actually has.
    pub async fn build_settlement_post(
        &self,
        payment_id: Uuid,
    ) -> Result<AccountingPostEnvelope, PaymentError> {
        // RLS scope (ADR-0008), ID-only: fenced by the request/inherited scope.
        let p = self
            .entries
            .fetch_post_source(&self.db_pool, payment_id)
            .await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if p.currency != "IDR" {
            return Err(PaymentError::UnsupportedCurrency(p.currency.clone()));
        }
        let rows = company_scope::with_company_scope(
            Some(p.company_id),
            self.allocations
                .fetch_for_payment(&self.db_pool, payment_id),
        )
        .await?;
        let allocs: Vec<AllocForPost> = rows
            .into_iter()
            .map(|r| AllocForPost {
                invoice_ref: r.invoice_ref,
                invoice_kind: r.invoice_kind,
                amount: r.allocated_amount,
                discount_amount: r.discount_amount,
                discount_account_id: r.discount_account_id,
            })
            .collect();
        Self::assemble_settlement_post(&p, payment_id, &allocs)
    }

    pub async fn post_payment(
        &self,
        payment_id: Uuid,
        sink: &dyn GlPostSink,
    ) -> Result<SettleOutcome, PaymentError> {
        if let Some(o) = self.short_circuit_posted(payment_id).await? {
            return Ok(o);
        }

        // Source + allocations, once: the landing computation and the discount resolution both read
        // this same fetched state.
        let p = self
            .entries
            .fetch_post_source(&self.db_pool, payment_id)
            .await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if p.currency != "IDR" {
            return Err(PaymentError::UnsupportedCurrency(p.currency.clone()));
        }
        // Pre-sink gate on the fused status: a rejected payment is terminal and a landed one was
        // short-circuited above, so anything not draft/submitted here refuses BEFORE the GL sink is
        // driven — otherwise a rejected payment could leave a journal the entry then disowns (the
        // CAS alone refuses only AFTER the sink has posted real money).
        if p.status != "draft" && p.status != "submitted" {
            return Err(PaymentError::NotPostable(p.status));
        }
        let rows = company_scope::with_company_scope(
            Some(p.company_id),
            self.allocations
                .fetch_for_payment(&self.db_pool, payment_id),
        )
        .await?;

        // Decide the discounts: a prior attempt's stamp is REUSED (idempotent retry after a GL
        // refusal must not re-open the window); an unstamped allocation resolves through the port —
        // applicable iff the invoice's materialized block says so on the posting date.
        let mut allocs: Vec<AllocForPost> = Vec::with_capacity(rows.len());
        let mut fresh: Vec<(Uuid, Decimal, Option<Uuid>)> = Vec::new(); // (allocation id, discount, account) to stamp
        for r in rows {
            let (discount, account, is_fresh) = if r.discount_amount > Decimal::ZERO {
                (r.discount_amount, r.discount_account_id, false)
            } else if let Some(d) = self
                .discount
                .resolve(p.company_id, r.invoice_ref, &r.invoice_kind, p.posting_date)
                .await?
            {
                // The basis clamp: an allocation may exceed the invoice's outstanding (payment
                // only bounds Σ allocations by the paid amount; billing knocks off the excess to
                // on-account AFTER this decision), so the discountable amount is the smaller of
                // the two — the cumulative take can never exceed percent × outstanding-at-resolve.
                (
                    super::payment_discount::discount_for(
                        r.allocated_amount.min(d.outstanding_basis),
                        &d,
                    ),
                    Some(d.account_id),
                    true,
                )
            } else {
                (Decimal::ZERO, None, false)
            };
            if is_fresh {
                fresh.push((r.id, discount, account));
            }
            allocs.push(AllocForPost {
                invoice_ref: r.invoice_ref,
                invoice_kind: r.invoice_kind,
                amount: r.allocated_amount,
                discount_amount: discount,
                discount_account_id: account,
            });
        }

        // The landing: reconcilability of the bank account × the channel dimension.
        let reconcilable = self
            .reconcilable
            .bank_reconcilable(&self.db_pool, p.company_id, p.bank_account_id)
            .await?;
        let landing = landing_state(reconcilable, &p.method).to_string();

        let env = Self::assemble_settlement_post(&p, payment_id, &allocs)?;

        match sink.post(&env).await {
            Ok(ack) => {
                // The stamps, the posted-transition (to `landing`), and the durable outbox stage
                // commit in ONE tx — a crash after the GL post cannot split the discount decision
                // from its journal, nor lose the `PaymentSettled` event.
                let mut tx = self.db_pool.begin().await?;
                company_scope::bind_company_on(&mut tx, env.company_id).await?;
                for (allocation_id, discount, account) in &fresh {
                    self.allocations
                        .stamp_discount(&mut tx, *allocation_id, *discount, *account)
                        .await?;
                }
                let rows_affected = self
                    .entries
                    .mark_posted(&mut tx, payment_id, ack.journal_id, ack.post_id, &landing)
                    .await?;
                if rows_affected == 0 {
                    tx.rollback().await?;
                    return self
                        .short_circuit_posted(payment_id)
                        .await?
                        .ok_or(PaymentError::PaymentNotFound(payment_id));
                }
                if let Some(schema) = self.outbox_schema.clone() {
                    self.stage_settled(&mut tx, &schema, payment_id, &env, &ack, &landing, &allocs)
                        .await?;
                }
                tx.commit().await?;
                self.emit_settled(payment_id, &env, &ack, &landing, &allocs)
                    .await?;
                Ok(SettleOutcome {
                    payment_id,
                    post_id: ack.post_id,
                    journal_id: ack.journal_id,
                    idempotent_reuse: ack.idempotent_reuse,
                })
            }
            Err(rej) => {
                // Deliberately ignored: the GL rejection below is the error being reported, and a
                // failure to mark the state must not mask it.
                let _ = self.entries.mark_failed(&self.db_pool, payment_id).await;
                Err(PaymentError::GlRejected {
                    code: rej.code,
                    message: rej.message,
                })
            }
        }
    }

    /// Stage `PaymentSettled` (contract v1.1: the landing status + the per-allocation discounts)
    /// into the durable outbox, reading the payment on the SAME transaction as the
    /// posted-transition so the event is atomic with the state change. The relay later delivers it;
    /// billing's `apply_settlements_once` dedups it.
    async fn stage_settled(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        schema: &str,
        payment_id: Uuid,
        env: &AccountingPostEnvelope,
        ack: &super::payment_gl::GlPostAck,
        status: &str,
        allocs: &[AllocForPost],
    ) -> Result<(), PaymentError> {
        let hdr = self
            .entries
            .fetch_type_and_amount_on(&mut **tx, payment_id)
            .await?;
        let payment_type: String = hdr.payment_type;
        let paid_amount: Decimal = hdr.paid_amount;
        let allocations: Vec<serde_json::Value> = allocs
            .iter()
            .map(|a| {
                let mut o = serde_json::json!({
                    "invoice_ref": a.invoice_ref.to_string(),
                    "invoice_kind": a.invoice_kind,
                    "amount": a.amount.to_string(),
                });
                if a.discount_amount > Decimal::ZERO {
                    o["discount_amount"] = serde_json::json!(a.discount_amount.to_string());
                }
                o
            })
            .collect();
        let payload = serde_json::json!({
            "payment_id": payment_id.to_string(),
            "company_id": env.company_id.to_string(),
            "payment_type": payment_type,
            "paid_amount": paid_amount.to_string(),
            "status": status,
            "journal_id": ack.journal_id.to_string(),
            "post_id": ack.post_id.to_string(),
            "allocations": allocations,
        });
        let rec = backbone_outbox::OutboxRecord::new(
            "PaymentSettled",
            "Payment",
            payment_id.to_string(),
            env.company_id,
            payload,
            chrono::Utc::now(),
        );
        backbone_outbox::outbox::stage(&mut **tx, schema, &rec)
            .await
            .map_err(|e| PaymentError::Db(sqlx::Error::Protocol(e.to_string())))?;
        Ok(())
    }

    async fn emit_settled(
        &self,
        payment_id: Uuid,
        env: &AccountingPostEnvelope,
        ack: &super::payment_gl::GlPostAck,
        status: &str,
        allocs: &[AllocForPost],
    ) -> Result<(), PaymentError> {
        let hdr = company_scope::with_company_scope(
            Some(env.company_id),
            self.entries.fetch_settled_header(&self.db_pool, payment_id),
        )
        .await?;
        let payment_type: String = hdr.payment_type;
        let paid_amount: Decimal = hdr.paid_amount;
        let unallocated: Decimal = hdr.unallocated_amount;
        let party_id: Option<Uuid> = hdr.party_id;

        let allocations: Vec<SettledInvoice> = allocs
            .iter()
            .map(|a| SettledInvoice {
                invoice_ref: a.invoice_ref,
                invoice_kind: a.invoice_kind.clone(),
                amount: a.amount,
                discount_amount: (a.discount_amount > Decimal::ZERO).then_some(a.discount_amount),
            })
            .collect();

        self.sink
            .publish(PaymentEvent::PaymentSettled(PaymentSettled {
                payment_id,
                company_id: env.company_id,
                journal_id: ack.journal_id,
                post_id: ack.post_id,
                payment_type,
                allocations,
                paid_amount,
                status: Some(status.to_string()),
                correlation_id: None,
                causation_id: None,
            }));
        if unallocated > Decimal::ZERO {
            self.sink.publish(PaymentEvent::PaymentReceivedOnAccount(
                PaymentReceivedOnAccount {
                    payment_id,
                    company_id: env.company_id,
                    party_id,
                    unallocated_amount: unallocated,
                },
            ));
        }
        Ok(())
    }

    async fn short_circuit_posted(
        &self,
        payment_id: Uuid,
    ) -> Result<Option<SettleOutcome>, PaymentError> {
        let row = self
            .entries
            .fetch_posted_state(&self.db_pool, payment_id)
            .await?
            .ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if row.posting_state == "posted" {
            if let (Some(j), Some(p)) = (row.journal_id, row.accounting_post_id) {
                return Ok(Some(SettleOutcome {
                    payment_id,
                    post_id: p,
                    journal_id: j,
                    idempotent_reuse: true,
                }));
            }
        }
        Ok(None)
    }
}
