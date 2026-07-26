use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "dunning_run_status", rename_all = "snake_case")]
pub enum DunningRunStatus {
    Building,
    Completed,
    Aborted,
}

impl std::fmt::Display for DunningRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Building => write!(f, "building"),
            Self::Completed => write!(f, "completed"),
            Self::Aborted => write!(f, "aborted"),
        }
    }
}

impl FromStr for DunningRunStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "building" => Ok(Self::Building),
            "completed" => Ok(Self::Completed),
            "aborted" => Ok(Self::Aborted),
            _ => Err(format!("Unknown DunningRunStatus variant: {}", s)),
        }
    }
}

impl Default for DunningRunStatus {
    fn default() -> Self {
        Self::Building
    }
}
