use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "dunning_action_type", rename_all = "snake_case")]
pub enum DunningActionType {
    SendReminder,
    SendOverdue,
    SendFinalNotice,
    EscalateToAgency,
    RecommendWriteOff,
}

impl std::fmt::Display for DunningActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SendReminder => write!(f, "send_reminder"),
            Self::SendOverdue => write!(f, "send_overdue"),
            Self::SendFinalNotice => write!(f, "send_final_notice"),
            Self::EscalateToAgency => write!(f, "escalate_to_agency"),
            Self::RecommendWriteOff => write!(f, "recommend_write_off"),
        }
    }
}

impl FromStr for DunningActionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "send_reminder" => Ok(Self::SendReminder),
            "send_overdue" => Ok(Self::SendOverdue),
            "send_final_notice" => Ok(Self::SendFinalNotice),
            "escalate_to_agency" => Ok(Self::EscalateToAgency),
            "recommend_write_off" => Ok(Self::RecommendWriteOff),
            _ => Err(format!("Unknown DunningActionType variant: {}", s)),
        }
    }
}

impl Default for DunningActionType {
    fn default() -> Self {
        Self::SendReminder
    }
}
