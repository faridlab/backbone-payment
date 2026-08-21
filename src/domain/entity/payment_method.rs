use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "payment_method", rename_all = "snake_case")]
pub enum PaymentMethod {
    Manual,
    BankTransfer,
    Cash,
    Cheque,
    Gateway,
}

impl std::fmt::Display for PaymentMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::BankTransfer => write!(f, "bank_transfer"),
            Self::Cash => write!(f, "cash"),
            Self::Cheque => write!(f, "cheque"),
            Self::Gateway => write!(f, "gateway"),
        }
    }
}

impl FromStr for PaymentMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "manual" => Ok(Self::Manual),
            "bank_transfer" => Ok(Self::BankTransfer),
            "cash" => Ok(Self::Cash),
            "cheque" => Ok(Self::Cheque),
            "gateway" => Ok(Self::Gateway),
            _ => Err(format!("Unknown PaymentMethod variant: {}", s)),
        }
    }
}

impl Default for PaymentMethod {
    fn default() -> Self {
        Self::Manual
    }
}
