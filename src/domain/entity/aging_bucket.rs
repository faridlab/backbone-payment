use chrono::{DateTime, Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::AgingBucketName;
use super::AuditMetadata;

/// Strongly-typed ID for AgingBucket
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgingBucketId(pub Uuid);

impl AgingBucketId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for AgingBucketId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AgingBucketId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for AgingBucketId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<AgingBucketId> for Uuid {
    fn from(id: AgingBucketId) -> Self { id.0 }
}

impl AsRef<Uuid> for AgingBucketId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for AgingBucketId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgingBucket {
    pub id: Uuid,
    pub company_id: Uuid,
    pub snapshot_id: Uuid,
    pub invoice_ref: Uuid,
    pub invoice_kind: String,
    pub party_id: Option<Uuid>,
    pub due_date: NaiveDate,
    pub days_past_due: i32,
    pub outstanding_amount: Decimal,
    pub bucket: AgingBucketName,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl AgingBucket {
    /// Create a builder for AgingBucket
    pub fn builder() -> AgingBucketBuilder {
        AgingBucketBuilder::default()
    }

    /// Create a new AgingBucket with required fields
    pub fn new(company_id: Uuid, snapshot_id: Uuid, invoice_ref: Uuid, invoice_kind: String, due_date: NaiveDate, days_past_due: i32, outstanding_amount: Decimal, bucket: AgingBucketName) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            snapshot_id,
            invoice_ref,
            invoice_kind,
            party_id: None,
            due_date,
            days_past_due,
            outstanding_amount,
            bucket,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> AgingBucketId {
        AgingBucketId(self.id)
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


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the party_id field (chainable)
    pub fn with_party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    // ==========================================================
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "company_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.company_id = v; }
                }
                "snapshot_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.snapshot_id = v; }
                }
                "invoice_ref" => {
                    if let Ok(v) = serde_json::from_value(value) { self.invoice_ref = v; }
                }
                "invoice_kind" => {
                    if let Ok(v) = serde_json::from_value(value) { self.invoice_kind = v; }
                }
                "party_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.party_id = v; }
                }
                "due_date" => {
                    if let Ok(v) = serde_json::from_value(value) { self.due_date = v; }
                }
                "days_past_due" => {
                    if let Ok(v) = serde_json::from_value(value) { self.days_past_due = v; }
                }
                "outstanding_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.outstanding_amount = v; }
                }
                "bucket" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for AgingBucket {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "AgingBucket"
    }
}

impl backbone_core::PersistentEntity for AgingBucket {
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

impl backbone_orm::EntityRepoMeta for AgingBucket {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("snapshot_id".to_string(), "uuid".to_string());
        m.insert("party_id".to_string(), "uuid".to_string());
        m.insert("bucket".to_string(), "aging_bucket_name".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["invoice_kind"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for AgingBucket entity
///
/// Provides a fluent API for constructing AgingBucket instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct AgingBucketBuilder {
    company_id: Option<Uuid>,
    snapshot_id: Option<Uuid>,
    invoice_ref: Option<Uuid>,
    invoice_kind: Option<String>,
    party_id: Option<Uuid>,
    due_date: Option<NaiveDate>,
    days_past_due: Option<i32>,
    outstanding_amount: Option<Decimal>,
    bucket: Option<AgingBucketName>,
}

impl AgingBucketBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the snapshot_id field (required)
    pub fn snapshot_id(mut self, value: Uuid) -> Self {
        self.snapshot_id = Some(value);
        self
    }

    /// Set the invoice_ref field (required)
    pub fn invoice_ref(mut self, value: Uuid) -> Self {
        self.invoice_ref = Some(value);
        self
    }

    /// Set the invoice_kind field (required)
    pub fn invoice_kind(mut self, value: String) -> Self {
        self.invoice_kind = Some(value);
        self
    }

    /// Set the party_id field (optional)
    pub fn party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the due_date field (required)
    pub fn due_date(mut self, value: NaiveDate) -> Self {
        self.due_date = Some(value);
        self
    }

    /// Set the days_past_due field (required)
    pub fn days_past_due(mut self, value: i32) -> Self {
        self.days_past_due = Some(value);
        self
    }

    /// Set the outstanding_amount field (required)
    pub fn outstanding_amount(mut self, value: Decimal) -> Self {
        self.outstanding_amount = Some(value);
        self
    }

    /// Set the bucket field (required)
    pub fn bucket(mut self, value: AgingBucketName) -> Self {
        self.bucket = Some(value);
        self
    }

    /// Build the AgingBucket entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<AgingBucket, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let snapshot_id = self.snapshot_id.ok_or_else(|| "snapshot_id is required".to_string())?;
        let invoice_ref = self.invoice_ref.ok_or_else(|| "invoice_ref is required".to_string())?;
        let invoice_kind = self.invoice_kind.ok_or_else(|| "invoice_kind is required".to_string())?;
        let due_date = self.due_date.ok_or_else(|| "due_date is required".to_string())?;
        let days_past_due = self.days_past_due.ok_or_else(|| "days_past_due is required".to_string())?;
        let outstanding_amount = self.outstanding_amount.ok_or_else(|| "outstanding_amount is required".to_string())?;
        let bucket = self.bucket.ok_or_else(|| "bucket is required".to_string())?;

        Ok(AgingBucket {
            id: Uuid::new_v4(),
            company_id,
            snapshot_id,
            invoice_ref,
            invoice_kind,
            party_id: self.party_id,
            due_date,
            days_past_due,
            outstanding_amount,
            bucket,
            metadata: AuditMetadata::default(),
        })
    }
}
