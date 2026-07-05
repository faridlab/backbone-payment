use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "payment_status", rename_all = "snake_case")]
pub enum PaymentStatus {
    Draft,
    Submitted,
    Posted,
    Cancelled,
}

impl std::fmt::Display for PaymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Submitted => write!(f, "submitted"),
            Self::Posted => write!(f, "posted"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for PaymentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "draft" => Ok(Self::Draft),
            "submitted" => Ok(Self::Submitted),
            "posted" => Ok(Self::Posted),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Unknown PaymentStatus variant: {}", s)),
        }
    }
}

impl Default for PaymentStatus {
    fn default() -> Self {
        Self::Draft
    }
}
