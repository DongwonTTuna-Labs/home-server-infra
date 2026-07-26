use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use gpt_webai_lifecycle::status::build_status;

#[test]
fn stopped_provider_limit_before_recheck_delay_is_not_allocatable() {
    let state_root = temp_state_root("provider-limit-recheck-pending");
    write_slot_state(
        &state_root,
        "status=provider.limit\nprovider_limit_next_retry_at_ms=9999999999999\n",
    );

    let status = build_status(&config(&state_root), &exited_runtime()).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "provider.limit");
    assert!(!slot_01.allocatable);
    assert_eq!(slot_01.persisted_status.as_deref(), Some("provider.limit"));
    let _ = std::fs::remove_dir_all(state_root);
}

#[test]
fn stopped_provider_limit_after_recheck_delay_becomes_standby_candidate() {
    let state_root = temp_state_root("provider-limit-recheck-ready");
    write_slot_state(
        &state_root,
        "status=provider.limit\nprovider_limit_next_retry_at_ms=1\n",
    );

    let status = build_status(&config(&state_root), &exited_runtime()).expect("status");
    let slot_01 = status.slots.first().expect("slot-01");
    assert_eq!(slot_01.status, "standby");
    assert!(slot_01.allocatable);
    assert_eq!(slot_01.persisted_status.as_deref(), Some("provider.limit"));
    let _ = std::fs::remove_dir_all(state_root);
}

fn write_slot_state(state_root: &std::path::Path, text: &str) {
    std::fs::create_dir_all(state_root.join("slots")).expect("slots dir");
    std::fs::write(state_root.join("slots").join("slot-01.state"), text).expect("slot state");
}

fn config(state_root: &std::path::Path) -> SupervisorConfig {
    SupervisorConfig {
        state_root: state_root.to_path_buf(),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    }
}

fn exited_runtime() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([(
        "slot-01".to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: None,
            provider_readiness: ProviderReadiness::NotChecked,
        },
    )])
}

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
