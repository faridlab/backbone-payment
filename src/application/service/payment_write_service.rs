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

use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

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
fn is_dup(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false)
}

#[derive(Clone)]
pub struct PaymentWriteService {
    db_pool: PgPool,
    sink: Arc<dyn PaymentEventSink>,
}

impl PaymentWriteService {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool, sink: Arc::new(LoggingSink) }
    }
    pub fn with_sink(db_pool: PgPool, sink: Arc<dyn PaymentEventSink>) -> Self {
        Self { db_pool, sink }
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

        let mut tx = self.db_pool.begin().await?;
        let r = sqlx::query(
            r#"INSERT INTO payment.payment_entries
                (id, payment_number, company_id, branch_id, payment_type, party_type, party_id,
                 posting_date, currency, mode_of_payment_id, paid_amount, allocated_amount,
                 unallocated_amount, bank_account_id, party_account_id, status, posting_state, reference_no)
               VALUES ($1,$2,$3,$4,$5::payment_type,$6::payment_party_type,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                       'draft'::payment_status,'pending'::gl_posting_state,$16)"#,
        )
        .bind(id).bind(&p.payment_number).bind(p.company_id).bind(p.branch_id).bind(&p.payment_type)
        .bind(&p.party_type).bind(p.party_id).bind(p.posting_date).bind(&currency).bind(p.mode_of_payment_id)
        .bind(paid).bind(allocated).bind(unallocated).bind(p.bank_account_id).bind(p.party_account_id)
        .bind(&p.reference_no)
        .execute(&mut *tx).await;
        if let Err(e) = r {
            return Err(if is_dup(&e) { PaymentError::DuplicateNumber(p.payment_number) } else { e.into() });
        }
        for a in &p.allocations {
            sqlx::query(
                r#"INSERT INTO payment.payment_allocations
                    (id, payment_id, invoice_ref, invoice_kind, allocated_amount)
                   VALUES ($1,$2,$3,$4::settlement_kind,$5)"#,
            )
            .bind(Uuid::new_v4()).bind(id).bind(a.invoice_ref).bind(&a.invoice_kind).bind(money(a.amount))
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    // ---- build the settlement post -----------------------------------------

    /// Build the balanced settlement post. receive: `Dr Bank (paid) · Cr A/R (paid) [customer]`;
    /// pay: `Dr A/P (paid) [supplier] · Cr Bank (paid)`. The A/R/A/P control is settled by the whole
    /// payment; any unallocated remainder sits as an on-account balance on that party (standard).
    pub async fn build_settlement_post(&self, payment_id: Uuid) -> Result<AccountingPostEnvelope, PaymentError> {
        let p = sqlx::query(
            r#"SELECT payment_number, company_id, branch_id, payment_type::text AS pt, party_type::text AS party_t,
                      party_id, posting_date, currency, paid_amount, bank_account_id, party_account_id
               FROM payment.payment_entries WHERE id=$1 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(payment_id).fetch_optional(&self.db_pool).await?.ok_or(PaymentError::PaymentNotFound(payment_id))?;
        let currency: String = p.get("currency");
        if currency != "IDR" { return Err(PaymentError::UnsupportedCurrency(currency)); }
        let payment_type: String = p.get("pt");
        let paid: Decimal = p.get("paid_amount");
        let number: String = p.get("payment_number");
        let bank: Uuid = p.get("bank_account_id");
        let control: Uuid = p.get("party_account_id");
        let party_id: Option<Uuid> = p.get("party_id");
        let party_type: Option<String> = p.get("party_t");

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
            idempotency_key: payment_id.to_string(), company_id: p.get("company_id"), branch_id: p.get("branch_id"),
            source_type: "payment".into(), source_id: payment_id, source_reference: Some(number),
            posting_date: p.get("posting_date"), currency, posting_type: "original".into(), reverses_post_id: None,
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
        let reverses_post_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT accounting_post_id FROM payment.payment_entries WHERE id=$1",
        ).bind(payment_id).fetch_one(&self.db_pool).await?;
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
                let res = sqlx::query(
                    r#"UPDATE payment.payment_entries SET posting_state='posted'::gl_posting_state,
                        status='posted'::payment_status, journal_id=$2, accounting_post_id=$3, posted_at=now()
                       WHERE id=$1 AND posting_state <> 'posted'::gl_posting_state"#,
                ).bind(payment_id).bind(ack.journal_id).bind(ack.post_id).execute(&self.db_pool).await?;
                if res.rows_affected() == 0 {
                    return self.short_circuit_posted(payment_id).await?
                        .ok_or(PaymentError::PaymentNotFound(payment_id));
                }
                self.emit_settled(payment_id, &env, &ack).await?;
                Ok(SettleOutcome { payment_id, post_id: ack.post_id, journal_id: ack.journal_id, idempotent_reuse: ack.idempotent_reuse })
            }
            Err(rej) => {
                let _ = sqlx::query("UPDATE payment.payment_entries SET posting_state='failed'::gl_posting_state WHERE id=$1").bind(payment_id).execute(&self.db_pool).await;
                Err(PaymentError::GlRejected { code: rej.code, message: rej.message })
            }
        }
    }

    async fn emit_settled(&self, payment_id: Uuid, env: &AccountingPostEnvelope, ack: &super::payment_gl::GlPostAck) -> Result<(), PaymentError> {
        let hdr = sqlx::query("SELECT payment_type::text AS pt, party_id, paid_amount, unallocated_amount FROM payment.payment_entries WHERE id=$1")
            .bind(payment_id).fetch_one(&self.db_pool).await?;
        let payment_type: String = hdr.get("pt");
        let paid_amount: Decimal = hdr.get("paid_amount");
        let unallocated: Decimal = hdr.get("unallocated_amount");
        let party_id: Option<Uuid> = hdr.get("party_id");

        let alloc_rows = sqlx::query("SELECT invoice_ref, invoice_kind::text AS kind, allocated_amount FROM payment.payment_allocations WHERE payment_id=$1 AND (metadata->>'deleted_at') IS NULL")
            .bind(payment_id).fetch_all(&self.db_pool).await?;
        let allocations: Vec<SettledInvoice> = alloc_rows.iter().map(|r| SettledInvoice {
            invoice_ref: r.get("invoice_ref"), invoice_kind: r.get("kind"), amount: r.get("allocated_amount"),
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
        let status: String = sqlx::query_scalar("SELECT status::text FROM payment.payment_entries WHERE id=$1 AND (metadata->>'deleted_at') IS NULL")
            .bind(payment_id).fetch_optional(&self.db_pool).await?.ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if status != "posted" && status != "cancelled" {
            return Err(PaymentError::NotReversible(status));
        }
        let env = self.build_reversal_post(payment_id).await?;
        match sink.post(&env).await {
            Ok(ack) => {
                let res = sqlx::query(
                    "UPDATE payment.payment_entries SET status='cancelled'::payment_status WHERE id=$1 AND status='posted'::payment_status",
                ).bind(payment_id).execute(&self.db_pool).await?;
                // Only the invocation that flipped posted→cancelled emits — so the reverse-seam restores
                // each invoice exactly once even under a repeat/concurrent reverse.
                if res.rows_affected() == 1 {
                    self.emit_cancelled(payment_id, &env, &ack).await?;
                }
                Ok(SettleOutcome {
                    payment_id, post_id: ack.post_id, journal_id: ack.journal_id,
                    idempotent_reuse: ack.idempotent_reuse || res.rows_affected() == 0,
                })
            }
            Err(rej) => Err(PaymentError::GlRejected { code: rej.code, message: rej.message }),
        }
    }

    async fn emit_cancelled(&self, payment_id: Uuid, env: &AccountingPostEnvelope, ack: &super::payment_gl::GlPostAck) -> Result<(), PaymentError> {
        let hdr = sqlx::query("SELECT payment_type::text AS pt, paid_amount FROM payment.payment_entries WHERE id=$1")
            .bind(payment_id).fetch_one(&self.db_pool).await?;
        let payment_type: String = hdr.get("pt");
        let paid_amount: Decimal = hdr.get("paid_amount");
        let alloc_rows = sqlx::query("SELECT invoice_ref, invoice_kind::text AS kind, allocated_amount FROM payment.payment_allocations WHERE payment_id=$1 AND (metadata->>'deleted_at') IS NULL")
            .bind(payment_id).fetch_all(&self.db_pool).await?;
        let allocations: Vec<SettledInvoice> = alloc_rows.iter().map(|r| SettledInvoice {
            invoice_ref: r.get("invoice_ref"), invoice_kind: r.get("kind"), amount: r.get("allocated_amount"),
        }).collect();
        self.sink.publish(PaymentEvent::PaymentCancelled(PaymentCancelled {
            payment_id, company_id: env.company_id, journal_id: ack.journal_id, post_id: ack.post_id,
            payment_type, allocations, paid_amount,
        }));
        Ok(())
    }

    // ---- shared -------------------------------------------------------------

    async fn short_circuit_posted(&self, payment_id: Uuid) -> Result<Option<SettleOutcome>, PaymentError> {
        let row = sqlx::query("SELECT posting_state::text AS ps, journal_id, accounting_post_id FROM payment.payment_entries WHERE id=$1 AND (metadata->>'deleted_at') IS NULL")
            .bind(payment_id).fetch_optional(&self.db_pool).await?.ok_or(PaymentError::PaymentNotFound(payment_id))?;
        if row.get::<String, _>("ps") == "posted" {
            if let (Some(j), Some(p)) = (row.get::<Option<Uuid>, _>("journal_id"), row.get::<Option<Uuid>, _>("accounting_post_id")) {
                return Ok(Some(SettleOutcome { payment_id, post_id: p, journal_id: j, idempotent_reuse: true }));
            }
        }
        Ok(None)
    }
}
