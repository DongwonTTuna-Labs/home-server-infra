use serde::Serialize;

use crate::contracts::health::HealthStatus;
use crate::slots::SlotConfig;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerStatus {
    Running,
    Exited,
    Missing,
    Unknown,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    Ready,
    ReadyModelCorrectionRequired,
    NotChecked,
    LoginRequired,
    SubscriptionRequired,
    ProviderLimit,
    Unreachable,
    SchemaDrift,
    Unknown,
}

impl ProviderReadiness {
    pub const fn from_health(status: HealthStatus) -> Self {
        match status {
            HealthStatus::Ready => Self::Ready,
            HealthStatus::ReadyModelCorrectionRequired => Self::ReadyModelCorrectionRequired,
            HealthStatus::LoginRequired => Self::LoginRequired,
            HealthStatus::SubscriptionRequired => Self::SubscriptionRequired,
            HealthStatus::ProviderLimit => Self::ProviderLimit,
            HealthStatus::Unreachable => Self::Unreachable,
            HealthStatus::SchemaDrift => Self::SchemaDrift,
            HealthStatus::Unknown => Self::Unknown,
        }
    }

    pub const fn health_status(&self) -> Option<HealthStatus> {
        match self {
            Self::Ready => Some(HealthStatus::Ready),
            Self::ReadyModelCorrectionRequired => Some(HealthStatus::ReadyModelCorrectionRequired),
            Self::NotChecked => None,
            Self::LoginRequired => Some(HealthStatus::LoginRequired),
            Self::SubscriptionRequired => Some(HealthStatus::SubscriptionRequired),
            Self::ProviderLimit => Some(HealthStatus::ProviderLimit),
            Self::Unreachable => Some(HealthStatus::Unreachable),
            Self::SchemaDrift => Some(HealthStatus::SchemaDrift),
            Self::Unknown => Some(HealthStatus::Unknown),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeObservation {
    pub docker_status: DockerStatus,
    pub cdp_reachable: Option<bool>,
    pub provider_readiness: ProviderReadiness,
}

pub trait RuntimeProbe {
    fn observe(&self, slot: &SlotConfig) -> RuntimeObservation;
}
