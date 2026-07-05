//! Outbound GL-posting port (hand-authored, user-owned) — payment side of the GL-posting contract.
//!
//! Payment is the settlement emitter: a receive posts `Dr Bank · Cr A/R [customer]`; a pay posts
//! `Dr A/P [supplier] · Cr Bank`. It emits a serialized `AccountingPostEnvelope` reached only through
//! a `GlPostSink`; the ACL adapter maps it into accounting's `PostingRequest`. Zero normal Cargo edge
//! (the envelope is the wire contract). Same shape as the selling/inventory/buying/billing ports —
//! the contract is duplicated per producer by design.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlPostLine {
    pub account_id: Uuid,
    pub debit: Decimal,
    pub credit: Decimal,
    pub party_type: Option<String>,
    pub party_id: Option<Uuid>,
    pub description: Option<String>,
}

impl GlPostLine {
    pub fn debit(account_id: Uuid, amount: Decimal) -> Self {
        Self { account_id, debit: amount, credit: Decimal::ZERO, party_type: None, party_id: None, description: None }
    }
    pub fn credit(account_id: Uuid, amount: Decimal) -> Self {
        Self { account_id, debit: Decimal::ZERO, credit: amount, party_type: None, party_id: None, description: None }
    }
    pub fn with_party(mut self, party_type: &str, party_id: Uuid) -> Self {
        self.party_type = Some(party_type.to_string());
        self.party_id = Some(party_id);
        self
    }
    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountingPostEnvelope {
    pub idempotency_key: String,
    pub company_id: Uuid,
    pub branch_id: Option<Uuid>,
    /// Posting source discriminator — "payment" (settlement).
    pub source_type: String,
    /// The producer document id (the payment id) — opaque to accounting.
    pub source_id: Uuid,
    pub source_reference: Option<String>,
    pub posting_date: chrono::NaiveDate,
    pub currency: String,
    /// "original" | "reversal".
    pub posting_type: String,
    /// The original post this reverses (set only when `posting_type == "reversal"`).
    pub reverses_post_id: Option<Uuid>,
    pub description: Option<String>,
    pub lines: Vec<GlPostLine>,
}

impl AccountingPostEnvelope {
    pub fn totals(&self) -> (Decimal, Decimal) {
        (self.lines.iter().map(|l| l.debit).sum(), self.lines.iter().map(|l| l.credit).sum())
    }
    pub fn is_balanced(&self) -> bool {
        let (d, c) = self.totals();
        d == c && !self.lines.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlPostAck {
    pub post_id: Uuid,
    pub journal_id: Uuid,
    pub idempotent_reuse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlPostRejected {
    pub code: String,
    pub message: String,
}

/// The settlement seam. A composing service implements this over accounting's `PostingService`.
#[async_trait::async_trait]
pub trait GlPostSink: Send + Sync {
    async fn post(&self, envelope: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected>;
}
