use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "payment_type", rename_all = "snake_case")]
pub enum PaymentType {
    Receive,
    Pay,
    InternalTransfer,
}

impl std::fmt::Display for PaymentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Receive => write!(f, "receive"),
            Self::Pay => write!(f, "pay"),
            Self::InternalTransfer => write!(f, "internal_transfer"),
        }
    }
}

impl FromStr for PaymentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "receive" => Ok(Self::Receive),
            "pay" => Ok(Self::Pay),
            "internal_transfer" => Ok(Self::InternalTransfer),
            _ => Err(format!("Unknown PaymentType variant: {}", s)),
        }
    }
}

impl Default for PaymentType {
    fn default() -> Self {
        Self::Receive
    }
}
