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

use backbone_orm::company_scope;
use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::infrastructure::persistence::{
    NewAllocationRow, NewPaymentEntryRow, PaymentAllocationRepository, PaymentEntryRepository,
};

use super::payment_events::{
    PaymentCancelled, PaymentEvent, PaymentEventSink, PaymentReceivedOnAccount, PaymentSettled,
    SettledInvoice, LoggingSink,
};
use super::payment_gl::{AccountingPostEnvelope, GlPostLine, GlPostSink};

fn money(v: Decimal) -> Decimal {
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
fn is_dup(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false)
}

/// The repositories are held behind `Arc` only so this service stays `Clone` —
/// `GenericCrudRepository` is not itself `Clone`. They are stateless handles over the same pool.
#[derive(Clone)]
pub struct PaymentWriteService {
    db_pool: PgPool,
    sink: Arc<dyn PaymentEventSink>,
    entries: Arc<PaymentEntryRepository>,
    allocations: Arc<PaymentAllocationRepository>,
    /// When set, `post_payment` stages `PaymentSettled` into `<schema>.outbox_events` **inside the
    /// posted-transition transaction** (crash-safe emission — go-live durable bus). When `None`, only the
    /// legacy in-proc sink fires (existing behaviour). The relay drains the outbox to the real bus.
    outbox_schema: Option<String>,
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

    // ---- create -------------------------------------------------------------

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
        }).await;
        if let Err(e) = r {
            return Err(if is_dup(&e) { PaymentError::DuplicateNumber(p.payment_number) } else { e.into() });
        }
        for a in &p.allocations {
            self.allocations.insert_allocation(&mut tx, &NewAllocationRow {
                id: Uuid::new_v4(),
                payment_id: id,
                invoice_ref: a.invoice_ref,
                invoice_kind: &a.invoice_kind,
                allocated_amount: money(a.amount),
            }).await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    // ---- build the settlement post -----------------------------------------

    /// Build the balanced settlement post. receive: `Dr Bank (paid) · Cr A/R (paid) [customer]`;
    /// pay: `Dr A/P (paid) [supplier] · Cr Bank (paid)`. The A/R/A/P control is settled by the whole
    /// payment; any unallocated remainder sits as an on-account balance on that party (standard).
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
                // Dr Bank · Cr A/R [customer]
                let mut ar = GlPostLine::credit(control, paid).with_description(format!("A/R settled {number}"));
                if let (Some(pt), Some(pid)) = (party_type.as_deref(), party_id) { ar = ar.with_party(pt, pid); }
                vec![GlPostLine::debit(bank, paid).with_description(format!("Receipt {number}")), ar]
            }
            "pay" => {
                // Dr A/P [supplier] · Cr Bank
                let mut ap = GlPostLine::debit(control, paid).with_description(format!("A/P settled {number}"));
                if let (Some(pt), Some(pid)) = (party_type.as_deref(), party_id) { ap = ap.with_party(pt, pid); }
                vec![ap, GlPostLine::credit(bank, paid).with_description(format!("Payment {number}"))]
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

    // ---- post ---------------------------------------------------------------

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
            "PaymentSettled", "Payment", payment_id.to_string(), payload, chrono::Utc::now());
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
        }));
        if unallocated > Decimal::ZERO {
            self.sink.publish(PaymentEvent::PaymentReceivedOnAccount(PaymentReceivedOnAccount {
                payment_id, company_id: env.company_id, party_id, unallocated_amount: unallocated,
            }));
        }
        Ok(())
    }

    // ---- reverse (refund / bounced cheque / mis-applied) --------------------

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
        }));
        Ok(())
    }

    // ---- shared -------------------------------------------------------------

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
