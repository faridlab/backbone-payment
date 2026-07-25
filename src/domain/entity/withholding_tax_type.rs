use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "withholding_tax_type", rename_all = "snake_case")]
pub enum WithholdingTaxType {
    None,
    Pph22,
    Pph23,
    Pph26,
}

impl std::fmt::Display for WithholdingTaxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Pph22 => write!(f, "pph_22"),
            Self::Pph23 => write!(f, "pph_23"),
            Self::Pph26 => write!(f, "pph_26"),
        }
    }
}

impl FromStr for WithholdingTaxType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "pph_22" => Ok(Self::Pph22),
            "pph_23" => Ok(Self::Pph23),
            "pph_26" => Ok(Self::Pph26),
            _ => Err(format!("Unknown WithholdingTaxType variant: {}", s)),
        }
    }
}

impl Default for WithholdingTaxType {
    fn default() -> Self {
        Self::None
    }
}
