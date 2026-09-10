//! The fused payment lifecycle: the hand verbs, the landing computation, and the bank-confirmation
//! drift consumer (hand-authored, user-owned).
//!
//! An `impl PaymentWriteService` chunk over the vocabulary in [`super::payment_write_service`]. The
//! state machine's contract lives on the `PaymentStatus` enum in the schema; this file is its only
//! writer set:
//!
//! - `submit_payment` / `reject_payment` — the hand verbs (CAS-guarded; a refused CAS reports the
//!   entry's actual state, never a silent no-op).
//! - [`landing_state`] — the pure computation `post_payment` uses to decide where a post lands:
//!   cash, or any payment against a NON-reconcilable bank account, lands `paid` (no bank statement
//!   will ever confirm it — waiting would be a stuck state); a reconcilable channel lands
//!   `in_flight` until the bank confirms.
//! - [`BankReconcilablePort`] — the reconcilability read, injected. The default reads
//!   `accounting.accounts.is_reconcilable` through a `to_regclass` guard: schema absent ⇒ the post
//!   REFUSES (fail-closed — the landing state cannot be computed safely), which is also what keeps
//!   the module's standalone tests honest: they inject a stub instead of silently defaulting.
//! - `confirm_cash_once` — the ONLY writer of in_flight→paid: the exactly-once consumer over
//!   `BankClearanceRecorded` (banking's outbox → payment's inbox). Lost liveness here is safe: the
//!   event is at-least-once redelivered, and a re-drift command re-invokes this consumer; a LOST
//!   event strands the status at in_flight, which is a stuck label, never wrong money — the GL and
//!   the invoice knock-offs committed with the post.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use backbone_orm::company_scope;
use backbone_orm::org_scope::{self, OrgScope};

use super::payment_write_service::{PaymentError, PaymentWriteService};

/// Where a post lands: cash (or a non-reconcilable bank account) is `paid` — there is no bank
/// statement coming to confirm it; a reconcilable channel is `in_flight` until the bank does.
/// Pure — the state-machine suite pins it.
pub fn landing_state(bank_reconcilable: bool, method: &str) -> &'static str {
    if method == "cash" || !bank_reconcilable {
        "paid"
    } else {
        "in_flight"
    }
}

/// Read whether a bank account is reconciliation-enabled in accounting. Injectable so module tests
/// (and non-accounting compositions) substitute a stub; the default is the real read.
#[async_trait::async_trait]
pub trait BankReconcilablePort: Send + Sync {
    async fn bank_reconcilable(
        &self,
        pool: &PgPool,
        company_id: Uuid,
        account_id: Uuid,
    ) -> Result<bool, PaymentError>;
}

/// The default reconcilability read: `accounting.accounts.is_reconcilable`, guarded with
/// `to_regclass`. A missing accounting schema (standalone module database, mis-composed host) REFUSES
/// the post rather than guessing a landing state — fail-closed, surfaced as
/// `reconcilable_probe_refused`.
pub struct AccountingReconcilableRead;

#[async_trait::async_trait]
impl BankReconcilablePort for AccountingReconcilableRead {
    async fn bank_reconcilable(
        &self,
        pool: &PgPool,
        company_id: Uuid,
        account_id: Uuid,
    ) -> Result<bool, PaymentError> {
        // Two-step probe: a bare `FROM accounting.accounts` against a missing schema is a plan-time
        // error no WHERE-clause guard can soften, so check `to_regclass` first (the tax-module
        // precedent). Schema absent ⇒ REFUSE — the landing state is unknowable, and guessing
        // (either direction) strands or silently confirms real money.
        let present: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('accounting.accounts')::text")
                .fetch_optional(pool)
                .await
                .map_err(PaymentError::Db)?
                .flatten();
        if present.is_none() {
            return Err(PaymentError::ReconcilableProbeRefused(
                "accounting.accounts is absent — the payment landing state cannot be computed"
                    .into(),
            ));
        }
        // RLS note: this read precedes the post unit of work, so it cannot ride the caller's tx —
        // `fetch_optional_row_scoped` opens its own short tx on the request-dedicated connection
        // and binds the org fence variables there. The predicate is id-only and the company is
        // carried by the scope: compositions that strip `company_id` from accounting (org-unit
        // axis, ADR-0029) have no company column to filter on, and their entitlement-union fence
        // reads `app.scope_unit_ids` — a legacy `app.company_id`-only bind leaves that empty and
        // the fence hides the row, which reads as "account absent" and refuses the post of an
        // otherwise valid payment. Compositions that still carry the company column stay correct
        // too: the scope also binds the legacy variable, so the company fence arm still matches.
        let row = org_scope::with_org_request_scope(
            pool,
            OrgScope::for_company_unit(company_id),
            org_scope::fetch_optional_row_scoped(
                pool,
                sqlx::query("SELECT is_reconcilable FROM accounting.accounts WHERE id=$1")
                    .bind(account_id),
            ),
        )
        .await
        .map_err(PaymentError::Db)?
        .map_err(PaymentError::Db)?;
        match row {
            None => Err(PaymentError::ReconcilableProbeRefused(format!(
                "bank account {account_id} is not readable in this company — account absent"
            ))),
            Some(r) => Ok(r.get("is_reconcilable")),
        }
    }
}

impl PaymentWriteService {
    /// Submit a draft payment — the hand verb that marks it ready to post. CAS on draft: a payment
    /// that already moved (submitted, landed, terminal) refuses with its actual state.
    pub async fn submit_payment(
        &self,
        company_id: Uuid,
        payment_id: Uuid,
    ) -> Result<(), PaymentError> {
        let affected = self
            .entries
            .mark_submitted(&self.db_pool, payment_id)
            .await?;
        if affected == 0 {
            // Distinguish "not yours / not there" from "refused in a non-draft state" — the operator
            // needs to know which. The status read is fence-correct: unscoped ⇒ not found.
            let status = self.fetch_status_scoped(company_id, payment_id).await?;
            return Err(PaymentError::NotSubmittable(status));
        }
        Ok(())
    }

    /// Reject a payment — the terminal operator exit for a payment that must not settle (fraud stop,
    /// countermand). CAS from submitted ONLY: rejecting is a pure label flip, legitimate only while
    /// nothing has committed. A draft must be discarded rather than rejected (nothing settled); an
    /// in_flight or paid entry has a committed journal and is exited by `reverse_payment`, which
    /// unwinds the GL — reject would strand the journal with no remaining exit.
    pub async fn reject_payment(
        &self,
        company_id: Uuid,
        payment_id: Uuid,
    ) -> Result<(), PaymentError> {
        let affected = self
            .entries
            .mark_rejected(&self.db_pool, payment_id)
            .await?;
        if affected == 0 {
            let status = self.fetch_status_scoped(company_id, payment_id).await?;
            return Err(PaymentError::NotRejectable(status));
        }
        Ok(())
    }

    /// The bank-confirmation drift, exactly-once: the consumer over `BankClearanceRecorded`. Dedups
    /// on the bus `event_id` at payment's inbox and CAS-flips in_flight→paid in the SAME transaction,
    /// so an at-least-once redelivery is a no-op and a crash between dedup and drift commits neither.
    /// Returns whether THIS invocation performed the drift (false = redelivery or non-applicable
    /// state — never an error; the event may legitimately name an already-paid or never-in-flight
    /// payment). Requires `backbone_outbox::outbox::migrate` to have created `payment.inbox_consumed`.
    pub async fn confirm_cash_once(
        &self,
        event_id: Uuid,
        consumer: &str,
        company_id: Uuid,
        payment_id: Uuid,
    ) -> Result<bool, PaymentError> {
        let mut tx = self.db_pool.begin().await?;
        // RLS scope: explicit company — the relay/ACL passes the event's company (the billing
        // consumer's precedent), never an ambient scope.
        company_scope::bind_company_on(&mut tx, company_id).await?;
        let first = backbone_outbox::inbox::once(&mut *tx, "payment", consumer, event_id)
            .await
            .map_err(|e| PaymentError::Db(sqlx::Error::Protocol(e.to_string())))?;
        if !first {
            tx.commit().await?; // already consumed — exactly-once no-op
            return Ok(false);
        }
        let drifted = self.entries.confirm_cash_on(&mut tx, payment_id).await?;
        tx.commit().await?;
        Ok(drifted == 1)
    }

    /// Status read under an explicit company scope (the verbs' refusal diagnostics).
    async fn fetch_status_scoped(
        &self,
        company_id: Uuid,
        payment_id: Uuid,
    ) -> Result<String, PaymentError> {
        company_scope::with_company_scope(
            Some(company_id),
            self.entries.fetch_status(&self.db_pool, payment_id),
        )
        .await?
        .map(|(status, _posting_state)| status)
        .ok_or(PaymentError::PaymentNotFound(payment_id))
    }
}
