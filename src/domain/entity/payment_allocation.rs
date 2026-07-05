use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use rust_decimal::Decimal;

use super::SettlementKind;
use super::AuditMetadata;

/// Strongly-typed ID for PaymentAllocation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentAllocationId(pub Uuid);

impl PaymentAllocationId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for PaymentAllocationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for PaymentAllocationId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for PaymentAllocationId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<PaymentAllocationId> for Uuid {
    fn from(id: PaymentAllocationId) -> Self { id.0 }
}

impl AsRef<Uuid> for PaymentAllocationId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for PaymentAllocationId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PaymentAllocation {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub invoice_ref: Uuid,
    pub invoice_kind: SettlementKind,
    pub allocated_amount: Decimal,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl PaymentAllocation {
    /// Create a builder for PaymentAllocation
    pub fn builder() -> PaymentAllocationBuilder {
        PaymentAllocationBuilder::default()
    }

    /// Create a new PaymentAllocation with required fields
    pub fn new(payment_id: Uuid, invoice_ref: Uuid, invoice_kind: SettlementKind, allocated_amount: Decimal) -> Self {
        Self {
            id: Uuid::new_v4(),
            payment_id,
            invoice_ref,
            invoice_kind,
            allocated_amount,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> PaymentAllocationId {
        PaymentAllocationId(self.id)
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
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "payment_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.payment_id = v; }
                }
                "invoice_ref" => {
                    if let Ok(v) = serde_json::from_value(value) { self.invoice_ref = v; }
                }
                "invoice_kind" => {
                    if let Ok(v) = serde_json::from_value(value) { self.invoice_kind = v; }
                }
                "allocated_amount" => {
                    if let Ok(v) = serde_json::from_value(value) { self.allocated_amount = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for PaymentAllocation {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "PaymentAllocation"
    }
}

impl backbone_core::PersistentEntity for PaymentAllocation {
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

impl backbone_orm::EntityRepoMeta for PaymentAllocation {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("payment_id".to_string(), "uuid".to_string());
        m.insert("invoice_kind".to_string(), "settlement_kind".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &[]
    }
    fn relations() -> &'static [(&'static str, &'static str, &'static str)] {
        &[("payment", "payment_entries", "paymentId")]
    }
}

/// Builder for PaymentAllocation entity
///
/// Provides a fluent API for constructing PaymentAllocation instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct PaymentAllocationBuilder {
    payment_id: Option<Uuid>,
    invoice_ref: Option<Uuid>,
    invoice_kind: Option<SettlementKind>,
    allocated_amount: Option<Decimal>,
}

impl PaymentAllocationBuilder {
    /// Set the payment_id field (required)
    pub fn payment_id(mut self, value: Uuid) -> Self {
        self.payment_id = Some(value);
        self
    }

    /// Set the invoice_ref field (required)
    pub fn invoice_ref(mut self, value: Uuid) -> Self {
        self.invoice_ref = Some(value);
        self
    }

    /// Set the invoice_kind field (required)
    pub fn invoice_kind(mut self, value: SettlementKind) -> Self {
        self.invoice_kind = Some(value);
        self
    }

    /// Set the allocated_amount field (required)
    pub fn allocated_amount(mut self, value: Decimal) -> Self {
        self.allocated_amount = Some(value);
        self
    }

    /// Build the PaymentAllocation entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<PaymentAllocation, String> {
        let payment_id = self.payment_id.ok_or_else(|| "payment_id is required".to_string())?;
        let invoice_ref = self.invoice_ref.ok_or_else(|| "invoice_ref is required".to_string())?;
        let invoice_kind = self.invoice_kind.ok_or_else(|| "invoice_kind is required".to_string())?;
        let allocated_amount = self.allocated_amount.ok_or_else(|| "allocated_amount is required".to_string())?;

        Ok(PaymentAllocation {
            id: Uuid::new_v4(),
            payment_id,
            invoice_ref,
            invoice_kind,
            allocated_amount,
            metadata: AuditMetadata::default(),
        })
    }
}
