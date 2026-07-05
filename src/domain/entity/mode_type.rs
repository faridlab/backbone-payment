use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "mode_type", rename_all = "snake_case")]
pub enum ModeType {
    Cash,
    BankTransfer,
    Card,
    EWallet,
    VirtualAccount,
    Qris,
}

impl std::fmt::Display for ModeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cash => write!(f, "cash"),
            Self::BankTransfer => write!(f, "bank_transfer"),
            Self::Card => write!(f, "card"),
            Self::EWallet => write!(f, "e_wallet"),
            Self::VirtualAccount => write!(f, "virtual_account"),
            Self::Qris => write!(f, "qris"),
        }
    }
}

impl FromStr for ModeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cash" => Ok(Self::Cash),
            "bank_transfer" => Ok(Self::BankTransfer),
            "card" => Ok(Self::Card),
            "e_wallet" => Ok(Self::EWallet),
            "virtual_account" => Ok(Self::VirtualAccount),
            "qris" => Ok(Self::Qris),
            _ => Err(format!("Unknown ModeType variant: {}", s)),
        }
    }
}

impl Default for ModeType {
    fn default() -> Self {
        Self::Cash
    }
}
