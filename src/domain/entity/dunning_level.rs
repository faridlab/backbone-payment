use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "dunning_level", rename_all = "snake_case")]
pub enum DunningLevel {
    Current,
    Reminder,
    Overdue,
    FinalNotice,
    Collection,
    WrittenOff,
}

impl std::fmt::Display for DunningLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => write!(f, "current"),
            Self::Reminder => write!(f, "reminder"),
            Self::Overdue => write!(f, "overdue"),
            Self::FinalNotice => write!(f, "final_notice"),
            Self::Collection => write!(f, "collection"),
            Self::WrittenOff => write!(f, "written_off"),
        }
    }
}

impl FromStr for DunningLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "current" => Ok(Self::Current),
            "reminder" => Ok(Self::Reminder),
            "overdue" => Ok(Self::Overdue),
            "final_notice" => Ok(Self::FinalNotice),
            "collection" => Ok(Self::Collection),
            "written_off" => Ok(Self::WrittenOff),
            _ => Err(format!("Unknown DunningLevel variant: {}", s)),
        }
    }
}

impl Default for DunningLevel {
    fn default() -> Self {
        Self::Current
    }
}
