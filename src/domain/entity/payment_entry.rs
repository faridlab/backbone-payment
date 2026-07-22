use chrono::{DateTime, Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::PaymentType;
use super::PaymentPartyType;
use super::PaymentStatus;
use super::GlPostingState;
use super::AuditMetadata;

/// Strongly-typed ID for PaymentEntry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentEntryId(pub Uuid);

impl PaymentEntryId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for PaymentEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for PaymentEntryId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for PaymentEntryId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<PaymentEntryId> for Uuid {
    fn from(id: PaymentEntryId) -> Self { id.0 }
}

impl AsRef<Uuid> for PaymentEntryId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for PaymentEntryId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PaymentEntry {
    pub id: Uuid,
    pub payment_number: String,
    pub company_id: Uuid,
    pub branch_id: Option<Uuid>,
    pub payment_type: PaymentType,
    pub party_type: Option<PaymentPartyType>,
    pub party_id: Option<Uuid>,
    pub posting_date: NaiveDate,
    pub currency: String,
    pub mode_of_payment_id: Option<Uuid>,
    pub paid_amount: Decimal,
    pub allocated_amount: Decimal,
    pub unallocated_amount: Decimal,
    pub bank_account_id: Uuid,
    pub party_account_id: Uuid,
    pub status: PaymentStatus,
    pub posting_state: GlPostingState,
    pub journal_id: Option<Uuid>,
    pub accounting_post_id: Option<Uuid>,
    pub posted_at: Option<DateTime<Utc>>,
    pub reference_no: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl PaymentEntry {
    /// Create a builder for PaymentEntry
    pub fn builder() -> PaymentEntryBuilder {
        PaymentEntryBuilder::default()
    }

    /// Create a new PaymentEntry with required fields
    pub fn new(payment_number: String, company_id: Uuid, payment_type: PaymentType, posting_date: NaiveDate, currency: String, paid_amount: Decimal, allocated_amount: Decimal, unallocated_amount: Decimal, bank_account_id: Uuid, party_account_id: Uuid, status: PaymentStatus, posting_state: GlPostingState) -> Self {
        Self {
            id: Uuid::new_v4(),
            payment_number,
            company_id,
            branch_id: None,
            payment_type,
            party_type: None,
            party_id: None,
            posting_date,
            currency,
            mode_of_payment_id: None,
            paid_amount,
            allocated_amount,
            unallocated_amount,
            bank_account_id,
            party_account_id,
            status,
            posting_state,
            journal_id: None,
            accounting_post_id: None,
            posted_at: None,
            reference_no: None,
            notes: None,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> PaymentEntryId {
        PaymentEntryId(self.id)
    }

    /// Get when this entity was created
    pub fn created_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.created_at.as_ref()
    }

    /// Get when this entity was last updated
    pub fn updated_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.updated_at.as_ref()
    }

    /// Check if this entity is soft deleted
    pub fn is_deleted(&self) -> bool {
        self.metadata.deleted_at.is_some()
    }

    /// Check if this entity is active (not deleted)
    pub fn is_active(&self) -> bool {
        self.metadata.deleted_at.is_none()
    }

    /// Get when this entity was deleted
    pub fn deleted_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.deleted_at.as_ref()
    }

    /// Get who created this entity
    pub fn created_by(&self) -> Option<&Uuid> {
        self.metadata.created_by.as_ref()
    }

    /// Get who last updated this entity
    pub fn updated_by(&self) -> Option<&Uuid> {
        self.metadata.updated_by.as_ref()
    }

    /// Get who deleted this entity
    pub fn deleted_by(&self) -> Option<&Uuid> {
        self.metadata.deleted_by.as_ref()
    }

    /// Get the current status
    pub fn status(&self) -> &PaymentStatus {
        &self.status
    }


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the branch_id field (chainable)
    pub fn with_branch_id(mut self, value: Uuid) -> Self {
        self.branch_id = Some(value);
        self
    }

    /// Set the party_type field (chainable)
    pub fn with_party_type(mut self, value: PaymentPartyType) -> Self {
        self.party_type = Some(value);
        self
    }

    /// Set the party_id field (chainable)
    pub fn with_party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the mode_of_payment_id field (chainable)
    pub fn with_mode_of_payment_id(mut self, value: Uuid) -> Self {
        self.mode_of_payment_id = Some(value);
        self
    }

    /// Set the journal_id field (chainable)
    pub fn with_journal_id(mut self, value: Uuid) -> Self {
        self.journal_id = Some(value);
        self
    }

    /// Set the accounting_post_id field (chainable)
    pub fn with_accounting_post_id(mut self, value: Uuid) -> Self {
        self.accounting_post_id = Some(value);
        self
    }

    /// Set the posted_at field (chainable)
    pub fn with_posted_at(mut self, value: DateTime<Utc>) -> Self {
        self.posted_at = Some(value);
        self
    }

    /// Set the reference_no field (chainable)
    pub fn with_reference_no(mut self, value: String) -> Self {
        self.reference_no = Some(value);
        self
    }

    /// Set the notes field (chainable)
    pub fn with_notes(mut self, value: String) -> Self {
        self.notes = Some(value);
        self
    }

    // ==========================================================
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "payment_number" => {
                    if let Ok(v) = serde_json::from_value(value) { self.payment_number = v; }
                }
                "company_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.company_id = v; }
                }
                "branch_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.branch_id = v; }
                }
                "payment_type" => {
                    if let Ok(v) = serde_json::from_value(value) { self.payment_type = v; }
                }
                "party_type" => {
                    if let Ok(v) = serde_json::from_value(value) { self.party_type = v; }
                }
                "party_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.party_id = v; }
                }
                "posting_date" => {
                    if let Ok(v) = serde_json::from_value(value) { self.posting_date = v; }
                }
                "currency" => {
                    if let Ok(v) = serde_json::from_value(value) { self.currency = v; }
                }
                "mode_of_payment_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.mode_of_payment_id = v; }
                }
                "paid_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.paid_amount = v; }
                }
                "allocated_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.allocated_amount = v; }
                }
                "unallocated_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.unallocated_amount = v; }
                }
                "bank_account_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bank_account_id = v; }
                }
                "party_account_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.party_account_id = v; }
                }
                "status" => {
                    if let Ok(v) = serde_json::from_value(value) { self.status = v; }
                }
                "posting_state" => {
                    if let Ok(v) = serde_json::from_value(value) { self.posting_state = v; }
                }
                "journal_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.journal_id = v; }
                }
                "accounting_post_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.accounting_post_id = v; }
                }
                "posted_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.posted_at = v; }
                }
                "reference_no" => {
                    if let Ok(v) = serde_json::from_value(value) { self.reference_no = v; }
                }
                "notes" => {
                    if let Ok(v) = serde_json::from_value(value) { self.notes = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for PaymentEntry {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "PaymentEntry"
    }
}

impl backbone_core::PersistentEntity for PaymentEntry {
    fn entity_id(&self) -> String {
        self.id.to_string()
    }
    fn set_entity_id(&mut self, id: String) {
        if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
            self.id = uuid;
        }
    }
    fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.created_at
    }
    fn set_created_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.created_at = Some(ts);
    }
    fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.updated_at
    }
    fn set_updated_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.updated_at = Some(ts);
    }
    fn deleted_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.deleted_at
    }
    fn set_deleted_at(&mut self, ts: Option<chrono::DateTime<chrono::Utc>>) {
        self.metadata.deleted_at = ts;
    }
}

impl backbone_orm::EntityRepoMeta for PaymentEntry {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("branch_id".to_string(), "uuid".to_string());
        m.insert("party_id".to_string(), "uuid".to_string());
        m.insert("mode_of_payment_id".to_string(), "uuid".to_string());
        m.insert("bank_account_id".to_string(), "uuid".to_string());
        m.insert("party_account_id".to_string(), "uuid".to_string());
        m.insert("journal_id".to_string(), "uuid".to_string());
        m.insert("accounting_post_id".to_string(), "uuid".to_string());
        m.insert("payment_type".to_string(), "payment_type".to_string());
        m.insert("party_type".to_string(), "payment_party_type".to_string());
        m.insert("status".to_string(), "payment_status".to_string());
        m.insert("posting_state".to_string(), "gl_posting_state".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["payment_number", "currency"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for PaymentEntry entity
///
/// Provides a fluent API for constructing PaymentEntry instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct PaymentEntryBuilder {
    payment_number: Option<String>,
    company_id: Option<Uuid>,
    branch_id: Option<Uuid>,
    payment_type: Option<PaymentType>,
    party_type: Option<PaymentPartyType>,
    party_id: Option<Uuid>,
    posting_date: Option<NaiveDate>,
    currency: Option<String>,
    mode_of_payment_id: Option<Uuid>,
    paid_amount: Option<Decimal>,
    allocated_amount: Option<Decimal>,
    unallocated_amount: Option<Decimal>,
    bank_account_id: Option<Uuid>,
    party_account_id: Option<Uuid>,
    status: Option<PaymentStatus>,
    posting_state: Option<GlPostingState>,
    journal_id: Option<Uuid>,
    accounting_post_id: Option<Uuid>,
    posted_at: Option<DateTime<Utc>>,
    reference_no: Option<String>,
    notes: Option<String>,
}

impl PaymentEntryBuilder {
    /// Set the payment_number field (required)
    pub fn payment_number(mut self, value: String) -> Self {
        self.payment_number = Some(value);
        self
    }

    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the branch_id field (optional)
    pub fn branch_id(mut self, value: Uuid) -> Self {
        self.branch_id = Some(value);
        self
    }

    /// Set the payment_type field (required)
    pub fn payment_type(mut self, value: PaymentType) -> Self {
        self.payment_type = Some(value);
        self
    }

    /// Set the party_type field (optional)
    pub fn party_type(mut self, value: PaymentPartyType) -> Self {
        self.party_type = Some(value);
        self
    }

    /// Set the party_id field (optional)
    pub fn party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the posting_date field (required)
    pub fn posting_date(mut self, value: NaiveDate) -> Self {
        self.posting_date = Some(value);
        self
    }

    /// Set the currency field (default: `"IDR".to_string()`)
    pub fn currency(mut self, value: String) -> Self {
        self.currency = Some(value);
        self
    }

    /// Set the mode_of_payment_id field (optional)
    pub fn mode_of_payment_id(mut self, value: Uuid) -> Self {
        self.mode_of_payment_id = Some(value);
        self
    }

    /// Set the paid_amount field (required)
    pub fn paid_amount(mut self, value: Decimal) -> Self {
        self.paid_amount = Some(value);
        self
    }

    /// Set the allocated_amount field (default: `Decimal::from(0)`)
    pub fn allocated_amount(mut self, value: Decimal) -> Self {
        self.allocated_amount = Some(value);
        self
    }

    /// Set the unallocated_amount field (default: `Decimal::from(0)`)
    pub fn unallocated_amount(mut self, value: Decimal) -> Self {
        self.unallocated_amount = Some(value);
        self
    }

    /// Set the bank_account_id field (required)
    pub fn bank_account_id(mut self, value: Uuid) -> Self {
        self.bank_account_id = Some(value);
        self
    }

    /// Set the party_account_id field (required)
    pub fn party_account_id(mut self, value: Uuid) -> Self {
        self.party_account_id = Some(value);
        self
    }

    /// Set the status field (default: `PaymentStatus::default()`)
    pub fn status(mut self, value: PaymentStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Set the posting_state field (default: `GlPostingState::default()`)
    pub fn posting_state(mut self, value: GlPostingState) -> Self {
        self.posting_state = Some(value);
        self
    }

    /// Set the journal_id field (optional)
    pub fn journal_id(mut self, value: Uuid) -> Self {
        self.journal_id = Some(value);
        self
    }

    /// Set the accounting_post_id field (optional)
    pub fn accounting_post_id(mut self, value: Uuid) -> Self {
        self.accounting_post_id = Some(value);
        self
    }

    /// Set the posted_at field (optional)
    pub fn posted_at(mut self, value: DateTime<Utc>) -> Self {
        self.posted_at = Some(value);
        self
    }

    /// Set the reference_no field (optional)
    pub fn reference_no(mut self, value: String) -> Self {
        self.reference_no = Some(value);
        self
    }

    /// Set the notes field (optional)
    pub fn notes(mut self, value: String) -> Self {
        self.notes = Some(value);
        self
    }

    /// Build the PaymentEntry entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<PaymentEntry, String> {
        let payment_number = self.payment_number.ok_or_else(|| "payment_number is required".to_string())?;
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let payment_type = self.payment_type.ok_or_else(|| "payment_type is required".to_string())?;
        let posting_date = self.posting_date.ok_or_else(|| "posting_date is required".to_string())?;
        let paid_amount = self.paid_amount.ok_or_else(|| "paid_amount is required".to_string())?;
        let bank_account_id = self.bank_account_id.ok_or_else(|| "bank_account_id is required".to_string())?;
        let party_account_id = self.party_account_id.ok_or_else(|| "party_account_id is required".to_string())?;

        Ok(PaymentEntry {
            id: Uuid::new_v4(),
            payment_number,
            company_id,
            branch_id: self.branch_id,
            payment_type,
            party_type: self.party_type,
            party_id: self.party_id,
            posting_date,
            currency: self.currency.unwrap_or("IDR".to_string()),
            mode_of_payment_id: self.mode_of_payment_id,
            paid_amount,
            allocated_amount: self.allocated_amount.unwrap_or(Decimal::from(0)),
            unallocated_amount: self.unallocated_amount.unwrap_or(Decimal::from(0)),
            bank_account_id,
            party_account_id,
            status: self.status.unwrap_or(PaymentStatus::default()),
            posting_state: self.posting_state.unwrap_or(GlPostingState::default()),
            journal_id: self.journal_id,
            accounting_post_id: self.accounting_post_id,
            posted_at: self.posted_at,
            reference_no: self.reference_no,
            notes: self.notes,
            metadata: AuditMetadata::default(),
        })
    }
}
