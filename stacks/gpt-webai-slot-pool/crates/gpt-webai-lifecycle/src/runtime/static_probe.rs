use std::collections::BTreeMap;

use crate::slots::SlotConfig;

use super::probe::{DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe};

#[derive(Clone, Debug)]
pub struct StaticRuntimeProbe {
    observations: BTreeMap<String, RuntimeObservation>,
    fallback: RuntimeObservation,
}

impl StaticRuntimeProbe {
    pub fn new(observations: impl IntoIterator<Item = (String, RuntimeObservation)>) -> Self {
        Self {
            observations: observations.into_iter().collect(),
            fallback: RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: None,
                provider_readiness: ProviderReadiness::NotChecked,
            },
        }
    }
}

impl RuntimeProbe for StaticRuntimeProbe {
    fn observe(&self, slot: &SlotConfig) -> RuntimeObservation {
        self.observations
            .get(&slot.slot_id.0)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }
}
