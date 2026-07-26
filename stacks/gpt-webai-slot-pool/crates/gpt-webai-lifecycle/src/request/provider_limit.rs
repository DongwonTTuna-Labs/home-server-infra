use std::collections::BTreeSet;
use std::time::Duration;

use crate::config::SupervisorConfig;
use crate::runtime::control::write_slot_status;
use crate::runtime::{DockerStatus, RuntimeProbe};
use crate::status::{self, SlotStatusView};

use super::output::RequestRunOutput;

const ACCOUNT_GROUP_COUNT: usize = 2;

#[derive(Clone, Debug, Default)]
pub(crate) struct ProviderLimitRetryState {
    limited_groups: BTreeSet<String>,
    retry_delays_ms: Vec<u64>,
}

impl ProviderLimitRetryState {
    pub(crate) fn record(&mut self, output: &RequestRunOutput) {
        if let Some(group) = output.account_group.as_deref() {
            self.limited_groups.insert(group.to_string());
        }
    }

    pub(crate) fn limited_groups(&self) -> &BTreeSet<String> {
        &self.limited_groups
    }

    pub(crate) fn should_try_another_group_now(&self) -> bool {
        !self.limited_groups.is_empty() && self.limited_groups.len() < ACCOUNT_GROUP_COUNT
    }

    pub(crate) fn next_cooldown(&self, delays: &[Duration]) -> Option<Duration> {
        delays.get(self.retry_delays_ms.len()).copied()
    }

    pub(crate) fn push_cooldown(&mut self, delay: Duration) {
        self.retry_delays_ms.push(retry_delay_ms(delay));
        self.limited_groups.clear();
    }

    pub(crate) fn retry_delays_ms(&self) -> &[u64] {
        &self.retry_delays_ms
    }
}

pub fn default_provider_limit_retry_delays() -> Vec<Duration> {
    vec![Duration::from_secs(900), Duration::from_secs(900)]
}

pub(crate) fn is_provider_limit(output: &RequestRunOutput) -> bool {
    output.reason.as_deref() == Some("provider.limit")
        || output.poll_status.as_deref() == Some("provider_limit")
        || output.send_status.as_deref() == Some("provider_limit")
        || (output.reason.as_deref() == Some("visual_gate.failed")
            && (output.message.contains("provider_limit")
                || output.message.contains("provider.limit")))
}

pub(crate) fn retry_delay_ms(delay: Duration) -> u64 {
    u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn reopen_limited_groups_for_retry(
    config: &SupervisorConfig,
    runtime: &dyn RuntimeProbe,
    limited_groups: &BTreeSet<String>,
) -> Result<(), String> {
    if limited_groups.is_empty() {
        return Ok(());
    }
    let status = status::build_status(config, runtime).map_err(|error| error.to_string())?;
    for slot in status.slots {
        if should_reopen_slot(&slot, limited_groups) {
            write_slot_status(config, &slot.slot_id, "standby")
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn should_reopen_slot(slot: &SlotStatusView, limited_groups: &BTreeSet<String>) -> bool {
    limited_groups.contains(&slot.account_group)
        && slot.persisted_status.as_deref() == Some("provider.limit")
        && matches!(
            slot.docker_status,
            DockerStatus::Exited | DockerStatus::Missing | DockerStatus::Skipped
        )
}
