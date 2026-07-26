use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "aging_bucket_name", rename_all = "snake_case")]
pub enum AgingBucketName {
    Current,
    Bucket130,
    Bucket3160,
    Bucket6190,
    Bucket90p,
}

impl std::fmt::Display for AgingBucketName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => write!(f, "current"),
            Self::Bucket130 => write!(f, "bucket_1_30"),
            Self::Bucket3160 => write!(f, "bucket_31_60"),
            Self::Bucket6190 => write!(f, "bucket_61_90"),
            Self::Bucket90p => write!(f, "bucket_90p"),
        }
    }
}

impl FromStr for AgingBucketName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "current" => Ok(Self::Current),
            "bucket_1_30" => Ok(Self::Bucket130),
            "bucket_31_60" => Ok(Self::Bucket3160),
            "bucket_61_90" => Ok(Self::Bucket6190),
            "bucket_90p" => Ok(Self::Bucket90p),
            _ => Err(format!("Unknown AgingBucketName variant: {}", s)),
        }
    }
}

impl Default for AgingBucketName {
    fn default() -> Self {
        Self::Current
    }
}
