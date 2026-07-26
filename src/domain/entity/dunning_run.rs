use chrono::{DateTime, Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::DunningRunStatus;
use super::AuditMetadata;

/// Strongly-typed ID for DunningRun
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DunningRunId(pub Uuid);

impl DunningRunId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for DunningRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for DunningRunId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for DunningRunId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<DunningRunId> for Uuid {
    fn from(id: DunningRunId) -> Self { id.0 }
}

impl AsRef<Uuid> for DunningRunId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for DunningRunId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DunningRun {
    pub id: Uuid,
    pub company_id: Uuid,
    pub as_of_date: NaiveDate,
    pub direction: String,
    pub snapshot_id: Option<Uuid>,
    pub actions_emitted: i32,
    pub status: DunningRunStatus,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl DunningRun {
    /// Create a builder for DunningRun
    pub fn builder() -> DunningRunBuilder {
        DunningRunBuilder::default()
    }

    /// Create a new DunningRun with required fields
    pub fn new(company_id: Uuid, as_of_date: NaiveDate, direction: String, actions_emitted: i32, status: DunningRunStatus) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            as_of_date,
            direction,
            snapshot_id: None,
            actions_emitted,
            status,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> DunningRunId {
        DunningRunId(self.id)
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
    pub fn status(&self) -> &DunningRunStatus {
        &self.status
    }


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the snapshot_id field (chainable)
    pub fn with_snapshot_id(mut self, value: Uuid) -> Self {
        self.snapshot_id = Some(value);
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
                "as_of_date" => {
                    if let Ok(v) = serde_json::from_value(value) { self.as_of_date = v; }
                }
                "direction" => {
                    if let Ok(v) = serde_json::from_value(value) { self.direction = v; }
                }
                "snapshot_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.snapshot_id = v; }
                }
                "actions_emitted" => {
                    if let Ok(v) = serde_json::from_value(value) { self.actions_emitted = v; }
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

impl super::Entity for DunningRun {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "DunningRun"
    }
}

impl backbone_core::PersistentEntity for DunningRun {
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

impl backbone_orm::EntityRepoMeta for DunningRun {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("snapshot_id".to_string(), "uuid".to_string());
        m.insert("status".to_string(), "dunning_run_status".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["direction"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for DunningRun entity
///
/// Provides a fluent API for constructing DunningRun instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct DunningRunBuilder {
    company_id: Option<Uuid>,
    as_of_date: Option<NaiveDate>,
    direction: Option<String>,
    snapshot_id: Option<Uuid>,
    actions_emitted: Option<i32>,
    status: Option<DunningRunStatus>,
}

impl DunningRunBuilder {
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

    /// Set the snapshot_id field (optional)
    pub fn snapshot_id(mut self, value: Uuid) -> Self {
        self.snapshot_id = Some(value);
        self
    }

    /// Set the actions_emitted field (default: `0`)
    pub fn actions_emitted(mut self, value: i32) -> Self {
        self.actions_emitted = Some(value);
        self
    }

    /// Set the status field (default: `DunningRunStatus::default()`)
    pub fn status(mut self, value: DunningRunStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Build the DunningRun entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<DunningRun, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let as_of_date = self.as_of_date.ok_or_else(|| "as_of_date is required".to_string())?;
        let direction = self.direction.ok_or_else(|| "direction is required".to_string())?;

        Ok(DunningRun {
            id: Uuid::new_v4(),
            company_id,
            as_of_date,
            direction,
            snapshot_id: self.snapshot_id,
            actions_emitted: self.actions_emitted.unwrap_or(0),
            status: self.status.unwrap_or(DunningRunStatus::default()),
            metadata: AuditMetadata::default(),
        })
    }
}
