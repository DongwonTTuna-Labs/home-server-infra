use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ready,
    ReadyModelCorrectionRequired,
    LoginRequired,
    SubscriptionRequired,
    ProviderLimit,
    Unreachable,
    SchemaDrift,
    Unknown,
}

impl HealthStatus {
    pub const ALL: [Self; 8] = [
        Self::Ready,
        Self::ReadyModelCorrectionRequired,
        Self::LoginRequired,
        Self::SubscriptionRequired,
        Self::ProviderLimit,
        Self::Unreachable,
        Self::SchemaDrift,
        Self::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ReadyModelCorrectionRequired => "ready_model_correction_required",
            Self::LoginRequired => "login_required",
            Self::SubscriptionRequired => "subscription_required",
            Self::ProviderLimit => "provider_limit",
            Self::Unreachable => "unreachable",
            Self::SchemaDrift => "schema_drift",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    pub const fn is_allocatable(self) -> bool {
        matches!(self, Self::Ready | Self::ReadyModelCorrectionRequired)
    }

    pub const fn needs_single_retry(self) -> bool {
        matches!(self, Self::Unreachable | Self::Unknown)
    }

    pub const fn is_authentication_block(self) -> bool {
        matches!(self, Self::LoginRequired | Self::SubscriptionRequired)
    }
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
