use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::DunningLevel;
use super::DunningActionType;
use super::DunningActionStatus;
use super::AuditMetadata;

/// Strongly-typed ID for DunningAction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DunningActionId(pub Uuid);

impl DunningActionId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for DunningActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for DunningActionId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for DunningActionId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<DunningActionId> for Uuid {
    fn from(id: DunningActionId) -> Self { id.0 }
}

impl AsRef<Uuid> for DunningActionId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for DunningActionId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DunningAction {
    pub id: Uuid,
    pub company_id: Uuid,
    pub run_id: Uuid,
    pub invoice_ref: Uuid,
    pub invoice_kind: String,
    pub party_id: Option<Uuid>,
    pub level: DunningLevel,
    pub action_type: DunningActionType,
    pub days_past_due: i32,
    pub outstanding_amount: Decimal,
    pub status: DunningActionStatus,
    pub processed_at: Option<DateTime<Utc>>,
    pub result_ref: Option<String>,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl DunningAction {
    /// Create a builder for DunningAction
    pub fn builder() -> DunningActionBuilder {
        DunningActionBuilder::default()
    }

    /// Create a new DunningAction with required fields
    pub fn new(company_id: Uuid, run_id: Uuid, invoice_ref: Uuid, invoice_kind: String, level: DunningLevel, action_type: DunningActionType, days_past_due: i32, outstanding_amount: Decimal, status: DunningActionStatus) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            run_id,
            invoice_ref,
            invoice_kind,
            party_id: None,
            level,
            action_type,
            days_past_due,
            outstanding_amount,
            status,
            processed_at: None,
            result_ref: None,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> DunningActionId {
        DunningActionId(self.id)
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
    pub fn status(&self) -> &DunningActionStatus {
        &self.status
    }


    // ==========================================================
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the party_id field (chainable)
    pub fn with_party_id(mut self, value: Uuid) -> Self {
        self.party_id = Some(value);
        self
    }

    /// Set the processed_at field (chainable)
    pub fn with_processed_at(mut self, value: DateTime<Utc>) -> Self {
        self.processed_at = Some(value);
        self
    }

    /// Set the result_ref field (chainable)
    pub fn with_result_ref(mut self, value: String) -> Self {
        self.result_ref = Some(value);
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
                "run_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.run_id = v; }
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
                "level" => {
                    if let Ok(v) = serde_json::from_value(value) { self.level = v; }
                }
                "action_type" => {
                    if let Ok(v) = serde_json::from_value(value) { self.action_type = v; }
                }
                "days_past_due" => {
                    if let Ok(v) = serde_json::from_value(value) { self.days_past_due = v; }
                }
                "outstanding_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.outstanding_amount = v; }
                }
                "status" => {
                    if let Ok(v) = serde_json::from_value(value) { self.status = v; }
                }
                "processed_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.processed_at = v; }
                }
                "result_ref" => {
                    if let Ok(v) = serde_json::from_value(value) { self.result_ref = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for DunningAction {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "DunningAction"
    }
}

impl backbone_core::PersistentEntity for DunningAction {
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

impl backbone_orm::EntityRepoMeta for DunningAction {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("run_id".to_string(), "uuid".to_string());
        m.insert("party_id".to_string(), "uuid".to_string());
        m.insert("level".to_string(), "dunning_level".to_string());
        m.insert("action_type".to_string(), "dunning_action_type".to_string());
        m.insert("status".to_string(), "dunning_action_status".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["invoice_kind"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for DunningAction entity
///
/// Provides a fluent API for constructing DunningAction instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct DunningActionBuilder {
    company_id: Option<Uuid>,
    run_id: Option<Uuid>,
    invoice_ref: Option<Uuid>,
    invoice_kind: Option<String>,
    party_id: Option<Uuid>,
    level: Option<DunningLevel>,
    action_type: Option<DunningActionType>,
    days_past_due: Option<i32>,
    outstanding_amount: Option<Decimal>,
    status: Option<DunningActionStatus>,
    processed_at: Option<DateTime<Utc>>,
    result_ref: Option<String>,
}

impl DunningActionBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the run_id field (required)
    pub fn run_id(mut self, value: Uuid) -> Self {
        self.run_id = Some(value);
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

    /// Set the level field (required)
    pub fn level(mut self, value: DunningLevel) -> Self {
        self.level = Some(value);
        self
    }

    /// Set the action_type field (required)
    pub fn action_type(mut self, value: DunningActionType) -> Self {
        self.action_type = Some(value);
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

    /// Set the status field (default: `DunningActionStatus::default()`)
    pub fn status(mut self, value: DunningActionStatus) -> Self {
        self.status = Some(value);
        self
    }

    /// Set the processed_at field (optional)
    pub fn processed_at(mut self, value: DateTime<Utc>) -> Self {
        self.processed_at = Some(value);
        self
    }

    /// Set the result_ref field (optional)
    pub fn result_ref(mut self, value: String) -> Self {
        self.result_ref = Some(value);
        self
    }

    /// Build the DunningAction entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<DunningAction, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let run_id = self.run_id.ok_or_else(|| "run_id is required".to_string())?;
        let invoice_ref = self.invoice_ref.ok_or_else(|| "invoice_ref is required".to_string())?;
        let invoice_kind = self.invoice_kind.ok_or_else(|| "invoice_kind is required".to_string())?;
        let level = self.level.ok_or_else(|| "level is required".to_string())?;
        let action_type = self.action_type.ok_or_else(|| "action_type is required".to_string())?;
        let days_past_due = self.days_past_due.ok_or_else(|| "days_past_due is required".to_string())?;
        let outstanding_amount = self.outstanding_amount.ok_or_else(|| "outstanding_amount is required".to_string())?;

        Ok(DunningAction {
            id: Uuid::new_v4(),
            company_id,
            run_id,
            invoice_ref,
            invoice_kind,
            party_id: self.party_id,
            level,
            action_type,
            days_past_due,
            outstanding_amount,
            status: self.status.unwrap_or(DunningActionStatus::default()),
            processed_at: self.processed_at,
            result_ref: self.result_ref,
            metadata: AuditMetadata::default(),
        })
    }
}
