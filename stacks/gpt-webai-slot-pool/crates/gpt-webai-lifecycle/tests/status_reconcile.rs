use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::locks::acquire_slot_lease;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use gpt_webai_lifecycle::status::{build_status, reconciled_slot_status};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "status_reconcile/provider_limit.rs"]
mod provider_limit;

fn temp_state_root(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("gpt-webai-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}

#[test]
fn persisted_ready_with_exited_runtime_is_never_ready() {
    let status = reconciled_slot_status(
        Some("ready"),
        &DockerStatus::Exited,
        &ProviderReadiness::Ready,
    );
    assert_eq!(status, "exited");
}

#[test]
fn persisted_standby_with_exited_runtime_is_intentional_standby() {
    let status = reconciled_slot_status(
        Some("standby"),
        &DockerStatus::Exited,
        &ProviderReadiness::NotChecked,
    );
    assert_eq!(status, "standby");
}

#[test]
fn exited_runtime_preserves_persisted_auth_needs_pro_state() {
    let status = reconciled_slot_status(
        Some("auth.needs_pro"),
        &DockerStatus::Exited,
        &ProviderReadiness::NotChecked,
    );
    assert_eq!(status, "auth.needs_pro");
}

#[test]
fn running_provider_ready_recovers_persisted_auth_needs_pro_state() {
    let state_root = temp_state_root("auth-needs-pro-recovered");
    std::fs::create_dir_all(state_root.join("slots")).expect("slots dir");
    std::fs::write(
        state_root.join("slots").join("slot-01.state"),
        "status=auth.needs_pro\n",
    )
    .expect("slot state");
    let config = SupervisorConfig {
        state_root: state_root.clone(),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };
    let runtime = StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::Ready,
        },
    )]);

    let status = build_status(&config, &runtime).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "ready");
    assert!(slot_01.allocatable);
    assert_eq!(slot_01.persisted_status.as_deref(), Some("auth.needs_pro"));
    let _ = std::fs::remove_dir_all(state_root);
}

#[test]
fn running_runtime_without_provider_readiness_is_not_ready() {
    let status = reconciled_slot_status(
        Some("ready"),
        &DockerStatus::Running,
        &ProviderReadiness::NotChecked,
    );
    assert_eq!(status, "warming");
}

#[test]
fn running_runtime_with_provider_limit_is_not_allocatable_ready() {
    let status = reconciled_slot_status(
        Some("ready"),
        &DockerStatus::Running,
        &ProviderReadiness::ProviderLimit,
    );
    assert_eq!(status, "provider.limit");
}

#[test]
fn live_status_reconciles_slot_10_exited() {
    let config = SupervisorConfig {
        state_root: std::path::PathBuf::from("/tmp/nonexistent-gpt-webai-test"),
        slot_count: 10,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };
    let runtime = StaticRuntimeProbe::new([(
        "slot-10".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: None,
            provider_readiness: ProviderReadiness::Ready,
        },
    )]);

    let status = build_status(&config, &runtime).expect("status");
    let slot_10 = status
        .slots
        .iter()
        .find(|slot| slot.slot_id == "slot-10")
        .expect("slot-10");
    assert_eq!(slot_10.account_group, "group-02");
    assert_eq!(slot_10.status, "exited");
    assert!(!slot_10.allocatable);
}

#[test]
fn live_status_keeps_intentionally_stopped_slot_standby() {
    let state_root = temp_state_root("standby-exited");
    std::fs::create_dir_all(state_root.join("slots")).expect("slots dir");
    std::fs::write(
        state_root.join("slots").join("slot-01.state"),
        "status=standby\n",
    )
    .expect("slot state");
    let config = SupervisorConfig {
        state_root: state_root.clone(),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };
    let runtime = StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: None,
            provider_readiness: ProviderReadiness::NotChecked,
        },
    )]);

    let status = build_status(&config, &runtime).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "standby");
    assert!(slot_01.allocatable);
    let _ = std::fs::remove_dir_all(state_root);
}

#[test]
fn live_status_requires_provider_readiness_for_ready() {
    let config = SupervisorConfig {
        state_root: std::path::PathBuf::from("/tmp/nonexistent-gpt-webai-test"),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };
    let runtime = StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::NotChecked,
        },
    )]);

    let status = build_status(&config, &runtime).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "warming");
    assert!(!slot_01.allocatable);
    assert_eq!(slot_01.provider_readiness, ProviderReadiness::NotChecked);
}

#[test]
fn active_slot_lease_makes_otherwise_ready_slot_not_allocatable() {
    let state_root = temp_state_root("leased-status");
    acquire_slot_lease(
        &state_root,
        "slot-01",
        "request-a",
        "run-a",
        "token-a",
        30_000,
    )
    .expect("lease");
    let config = SupervisorConfig {
        state_root: state_root.clone(),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };
    let runtime = StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::Ready,
        },
    )]);

    let status = build_status(&config, &runtime).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "leased");
    assert!(!slot_01.allocatable);
    let _ = std::fs::remove_dir_all(state_root);
}
