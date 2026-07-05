use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::ModeType;
use super::AuditMetadata;

/// Strongly-typed ID for ModeOfPayment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModeOfPaymentId(pub Uuid);

impl ModeOfPaymentId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for ModeOfPaymentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ModeOfPaymentId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for ModeOfPaymentId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<ModeOfPaymentId> for Uuid {
    fn from(id: ModeOfPaymentId) -> Self { id.0 }
}

impl AsRef<Uuid> for ModeOfPaymentId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for ModeOfPaymentId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ModeOfPayment {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub mode_type: ModeType,
    pub default_account_id: Option<Uuid>,
    pub is_active: bool,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl ModeOfPayment {
    /// Create a builder for ModeOfPayment
    pub fn builder() -> ModeOfPaymentBuilder {
        ModeOfPaymentBuilder::default()
    }

    /// Create a new ModeOfPayment with required fields
    pub fn new(code: String, name: String, mode_type: ModeType, is_active: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            code,
            name,
            mode_type,
            default_account_id: None,
            is_active,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> ModeOfPaymentId {
        ModeOfPaymentId(self.id)
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

    /// Set the default_account_id field (chainable)
    pub fn with_default_account_id(mut self, value: Uuid) -> Self {
        self.default_account_id = Some(value);
        self
    }

    // ==========================================================
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "code" => {
                    if let Ok(v) = serde_json::from_value(value) { self.code = v; }
                }
                "name" => {
                    if let Ok(v) = serde_json::from_value(value) { self.name = v; }
                }
                "mode_type" => {
                    if let Ok(v) = serde_json::from_value(value) { self.mode_type = v; }
                }
                "default_account_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.default_account_id = v; }
                }
                "is_active" => {
                    if let Ok(v) = serde_json::from_value(value) { self.is_active = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for ModeOfPayment {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "ModeOfPayment"
    }
}

impl backbone_core::PersistentEntity for ModeOfPayment {
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

impl backbone_orm::EntityRepoMeta for ModeOfPayment {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("default_account_id".to_string(), "uuid".to_string());
        m.insert("mode_type".to_string(), "mode_type".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["code", "name"]
    }
}

/// Builder for ModeOfPayment entity
///
/// Provides a fluent API for constructing ModeOfPayment instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct ModeOfPaymentBuilder {
    code: Option<String>,
    name: Option<String>,
    mode_type: Option<ModeType>,
    default_account_id: Option<Uuid>,
    is_active: Option<bool>,
}

impl ModeOfPaymentBuilder {
    /// Set the code field (required)
    pub fn code(mut self, value: String) -> Self {
        self.code = Some(value);
        self
    }

    /// Set the name field (required)
    pub fn name(mut self, value: String) -> Self {
        self.name = Some(value);
        self
    }

    /// Set the mode_type field (required)
    pub fn mode_type(mut self, value: ModeType) -> Self {
        self.mode_type = Some(value);
        self
    }

    /// Set the default_account_id field (optional)
    pub fn default_account_id(mut self, value: Uuid) -> Self {
        self.default_account_id = Some(value);
        self
    }

    /// Set the is_active field (default: `true`)
    pub fn is_active(mut self, value: bool) -> Self {
        self.is_active = Some(value);
        self
    }

    /// Build the ModeOfPayment entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<ModeOfPayment, String> {
        let code = self.code.ok_or_else(|| "code is required".to_string())?;
        let name = self.name.ok_or_else(|| "name is required".to_string())?;
        let mode_type = self.mode_type.ok_or_else(|| "mode_type is required".to_string())?;

        Ok(ModeOfPayment {
            id: Uuid::new_v4(),
            code,
            name,
            mode_type,
            default_account_id: self.default_account_id,
            is_active: self.is_active.unwrap_or(true),
            metadata: AuditMetadata::default(),
        })
    }
}
