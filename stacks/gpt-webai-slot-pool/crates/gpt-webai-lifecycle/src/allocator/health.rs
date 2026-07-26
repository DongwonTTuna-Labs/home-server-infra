use crate::contracts::health::HealthStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthDecision {
    pub allocatable: bool,
    pub retry_after_ms: Option<u64>,
    pub cooldown_ms: u64,
}

pub fn map_health(status: HealthStatus, provider_retry_after_ms: Option<u64>) -> HealthDecision {
    match status {
        HealthStatus::Ready | HealthStatus::ReadyModelCorrectionRequired => HealthDecision {
            allocatable: true,
            retry_after_ms: None,
            cooldown_ms: 0,
        },
        HealthStatus::LoginRequired => blocked(None, 300_000),
        HealthStatus::SubscriptionRequired => blocked(None, 3_600_000),
        HealthStatus::ProviderLimit => blocked(
            None,
            provider_retry_after_ms
                .unwrap_or(300_000)
                .clamp(60_000, 3_600_000),
        ),
        HealthStatus::Unreachable => blocked(Some(250), 30_000),
        HealthStatus::SchemaDrift => blocked(None, 300_000),
        HealthStatus::Unknown => blocked(Some(250), 60_000),
    }
}

pub fn picker_failure_cooldown_ms() -> u64 {
    300_000
}

pub const fn status_result_kind(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Ready | HealthStatus::ReadyModelCorrectionRequired => "status.ready",
        HealthStatus::LoginRequired | HealthStatus::SubscriptionRequired => "status.blocked",
        HealthStatus::ProviderLimit
        | HealthStatus::Unreachable
        | HealthStatus::SchemaDrift
        | HealthStatus::Unknown => "status.degraded",
    }
}

fn blocked(retry_after_ms: Option<u64>, cooldown_ms: u64) -> HealthDecision {
    HealthDecision {
        allocatable: false,
        retry_after_ms,
        cooldown_ms,
    }
}
