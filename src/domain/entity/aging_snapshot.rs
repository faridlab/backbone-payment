use chrono::{DateTime, Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::SnapshotStatus;
use super::AuditMetadata;

/// Strongly-typed ID for AgingSnapshot
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgingSnapshotId(pub Uuid);

impl AgingSnapshotId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for AgingSnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AgingSnapshotId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for AgingSnapshotId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<AgingSnapshotId> for Uuid {
    fn from(id: AgingSnapshotId) -> Self { id.0 }
}

impl AsRef<Uuid> for AgingSnapshotId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for AgingSnapshotId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgingSnapshot {
    pub id: Uuid,
    pub company_id: Uuid,
    pub as_of_date: NaiveDate,
    pub direction: String,
    pub total_outstanding: Decimal,
    pub bucket_current: Decimal,
    pub bucket_1_30: Decimal,
    pub bucket_31_60: Decimal,
    pub bucket_61_90: Decimal,
    pub bucket_90p: Decimal,
    pub status: SnapshotStatus,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl AgingSnapshot {
    /// Create a builder for AgingSnapshot
    pub fn builder() -> AgingSnapshotBuilder {
        AgingSnapshotBuilder::default()
    }

    /// Create a new AgingSnapshot with required fields
    pub fn new(company_id: Uuid, as_of_date: NaiveDate, direction: String, total_outstanding: Decimal, bucket_current: Decimal, bucket_1_30: Decimal, bucket_31_60: Decimal, bucket_61_90: Decimal, bucket_90p: Decimal, status: SnapshotStatus) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            as_of_date,
            direction,
            total_outstanding,
            bucket_current,
            bucket_1_30,
            bucket_31_60,
            bucket_61_90,
            bucket_90p,
            status,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> AgingSnapshotId {
        AgingSnapshotId(self.id)
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
    pub fn status(&self) -> &SnapshotStatus {
        &self.status
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
                "as_of_date" => {
                    if let Ok(v) = serde_json::from_value(value) { self.as_of_date = v; }
                }
                "direction" => {
                    if let Ok(v) = serde_json::from_value(value) { self.direction = v; }
                }
                "total_outstanding" => {
                    if let Ok(v) = serde_json::from_value(value) { self.total_outstanding = v; }
                }
                "bucket_current" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket_current = v; }
                }
                "bucket_1_30" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket_1_30 = v; }
                }
                "bucket_31_60" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket_31_60 = v; }
                }
                "bucket_61_90" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket_61_90 = v; }
                }
                "bucket_90p" => {
                    if let Ok(v) = serde_json::from_value(value) { self.bucket_90p = v; }
                }
                "status" => {
                    if let Ok(v) = serde_json::from_value(value) { self.status = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for AgingSnapshot {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "AgingSnapshot"
    }
}

impl backbone_core::PersistentEntity for AgingSnapshot {
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

impl backbone_orm::EntityRepoMeta for AgingSnapshot {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("status".to_string(), "snapshot_status".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["direction"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for AgingSnapshot entity
///
/// Provides a fluent API for constructing AgingSnapshot instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct AgingSnapshotBuilder {
    company_id: Option<Uuid>,
    as_of_date: Option<NaiveDate>,
    direction: Option<String>,
    total_outstanding: Option<Decimal>,
    bucket_current: Option<Decimal>,
    bucket_1_30: Option<Decimal>,
    bucket_31_60: Option<Decimal>,
    bucket_61_90: Option<Decimal>,
    bucket_90p: Option<Decimal>,
    status: Option<SnapshotStatus>,
}

impl AgingSnapshotBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the as_of_date field (required)
    pub fn as_of_date(mut self, value: NaiveDate) -> Self {
        self.as_of_date = Some(value);
        self
    }

    /// Set the direction field (required)
    pub fn direction(mut self, value: String) -> Self {
        self.direction = Some(value);
        self
    }

    /// Set the total_outstanding field (default: `Decimal::from(0)`)
    pub fn total_outstanding(mut self, value: Decimal) -> Self {
        self.total_outstanding = Some(value);
        self
    }

    /// Set the bucket_current field (default: `Decimal::from(0)`)
    pub fn bucket_current(mut self, value: Decimal) -> Self {
        self.bucket_current = Some(value);
        self
    }

    /// Set the bucket_1_30 field (default: `Decimal::from(0)`)
    pub fn bucket_1_30(mut self, value: Decimal) -> Self {
        self.bucket_1_30 = Some(value);
        self
    }

    /// Set the bucket_31_60 field (default: `Decimal::from(0)`)
    pub fn bucket_31_60(mut self, value: Decimal) -> Self {
        self.bucket_31_60 = Some(value);
        self
    }

    /// Set the bucket_61_90 field (default: `Decimal::from(0)`)
    pub fn bucket_61_90(mut self, value: Decimal) -> Self {
        self.bucket_61_90 = Some(value);
        self
    }

    /// Set the bucket_90p field (default: `Decimal::from(0)`)
    pub fn bucket_90p(mut self, value: Decimal) -> Self {
        self.bucket_90p = Some(value);
        self
    }

    /// Set the status field (default: `SnapshotStatus::default()`)
    pub fn status(mut self, value: SnapshotStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Build the AgingSnapshot entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<AgingSnapshot, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let as_of_date = self.as_of_date.ok_or_else(|| "as_of_date is required".to_string())?;
        let direction = self.direction.ok_or_else(|| "direction is required".to_string())?;

        Ok(AgingSnapshot {
            id: Uuid::new_v4(),
            company_id,
            as_of_date,
            direction,
            total_outstanding: self.total_outstanding.unwrap_or(Decimal::from(0)),
            bucket_current: self.bucket_current.unwrap_or(Decimal::from(0)),
            bucket_1_30: self.bucket_1_30.unwrap_or(Decimal::from(0)),
            bucket_31_60: self.bucket_31_60.unwrap_or(Decimal::from(0)),
            bucket_61_90: self.bucket_61_90.unwrap_or(Decimal::from(0)),
            bucket_90p: self.bucket_90p.unwrap_or(Decimal::from(0)),
            status: self.status.unwrap_or(SnapshotStatus::default()),
            metadata: AuditMetadata::default(),
        })
    }
}
