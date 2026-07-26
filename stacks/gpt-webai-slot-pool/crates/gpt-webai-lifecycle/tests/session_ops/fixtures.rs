use std::cell::RefCell;

use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe, StaticRuntimeProbe,
};

pub(super) fn standby_exited_runtime() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: Some(false),
            provider_readiness: ProviderReadiness::NotChecked,
        },
    )])
}

pub(super) fn exited_then_ready_runtime() -> SequencedRuntimeProbe {
    SequencedRuntimeProbe::new([
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: Some(false),
            provider_readiness: ProviderReadiness::NotChecked,
        },
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::Ready,
        },
    ])
}

pub(super) struct SequencedRuntimeProbe {
    observations: RefCell<Vec<RuntimeObservation>>,
    fallback: RuntimeObservation,
}

impl SequencedRuntimeProbe {
    fn new(observations: impl IntoIterator<Item = RuntimeObservation>) -> Self {
        Self {
            observations: RefCell::new(observations.into_iter().collect()),
            fallback: RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        }
    }
}

impl RuntimeProbe for SequencedRuntimeProbe {
    fn observe(&self, _slot: &gpt_webai_lifecycle::slots::SlotConfig) -> RuntimeObservation {
        let mut observations = self.observations.borrow_mut();
        if observations.is_empty() {
            return self.fallback.clone();
        }
        observations.remove(0)
    }
}
