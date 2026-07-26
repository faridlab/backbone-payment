use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "snapshot_status", rename_all = "snake_case")]
pub enum SnapshotStatus {
    Building,
    Final,
    Aborted,
}

impl std::fmt::Display for SnapshotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Building => write!(f, "building"),
            Self::Final => write!(f, "final"),
            Self::Aborted => write!(f, "aborted"),
        }
    }
}

impl FromStr for SnapshotStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "building" => Ok(Self::Building),
            "final" => Ok(Self::Final),
            "aborted" => Ok(Self::Aborted),
            _ => Err(format!("Unknown SnapshotStatus variant: {}", s)),
        }
    }
}

impl Default for SnapshotStatus {
    fn default() -> Self {
        Self::Building
    }
}
