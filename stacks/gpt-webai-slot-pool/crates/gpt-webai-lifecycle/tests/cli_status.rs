use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::contracts::events::Writer;
use gpt_webai_lifecycle::contracts::projection::{CasRecord, SlotRecord};
use gpt_webai_lifecycle::journal::projection::empty_state;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use gpt_webai_lifecycle::slots::SlotConfig;
use gpt_webai_lifecycle::status::aggregate_r13_status;

#[test]
fn r13_status_is_invariant_to_legacy_slot_count() {
    let state = empty_state().expect("empty projection");
    let runtime = StaticRuntimeProbe::new([]);
    let one = aggregate_r13_status(&config(1), &runtime, &state);
    let seven = aggregate_r13_status(&config(7), &runtime, &state);

    assert_eq!(one.result_kind, "status.ready");
    assert_eq!(one.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(one.result_kind, seven.result_kind);
    assert_eq!(one.slot_id, seven.slot_id);
}

#[test]
fn auth_only_pool_is_blocked_but_mixed_unknown_pool_is_degraded() {
    let mut state = empty_state().expect("empty projection");
    for slot_id in gpt_webai_lifecycle::allocator::CANONICAL_SLOTS {
        state
            .slots
            .insert(slot_id.to_string(), slot_record(slot_id, "login_required"));
    }
    let runtime = StaticRuntimeProbe::new([]);
    let blocked = aggregate_r13_status(&config(10), &runtime, &state);
    assert_eq!(blocked.result_kind, "status.blocked");
    assert_eq!(blocked.slot_id.as_deref(), Some("slot-01"));

    state
        .slots
        .get_mut("slot-04")
        .expect("slot-04")
        .health_status = "unknown".to_string();
    let degraded = aggregate_r13_status(&config(10), &runtime, &state);
    assert_eq!(degraded.result_kind, "status.degraded");
    assert_eq!(degraded.slot_id.as_deref(), Some("slot-04"));
}

#[test]
fn failed_runtime_observations_without_recorded_health_are_probe_failed() {
    let mut state = empty_state().expect("empty projection");
    let mut observations = BTreeMap::new();
    for (index, slot_id) in gpt_webai_lifecycle::allocator::CANONICAL_SLOTS
        .into_iter()
        .enumerate()
    {
        state.leases.insert(
            format!("lease-{index}"),
            active_lease(slot_id, u16::try_from(index + 1).expect("generation")),
        );
        observations.insert(
            slot_id.to_string(),
            RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: Some(false),
                provider_readiness: ProviderReadiness::Unreachable,
            },
        );
    }
    let runtime = StaticRuntimeProbe::new(observations);
    let decision = aggregate_r13_status(&config(3), &runtime, &state);

    assert_eq!(decision.result_kind, "status.runtime_probe_failed");
    assert_eq!(
        decision.reason.as_deref(),
        Some("status.runtime_probe_failed")
    );
    assert_eq!(decision.slot_id.as_deref(), Some("slot-01"));
}

#[test]
fn direct_status_probe_retries_unreachable_once_and_uses_the_post_retry_value() {
    let mut state = empty_state().expect("empty projection");
    for (index, slot_id) in gpt_webai_lifecycle::allocator::CANONICAL_SLOTS
        .into_iter()
        .enumerate()
    {
        state.leases.insert(
            format!("lease-{index}"),
            active_lease(slot_id, u16::try_from(index + 1).expect("generation")),
        );
    }
    let runtime = RetryRuntimeProbe::default();
    let decision = aggregate_r13_status(&config(10), &runtime, &state);

    assert_eq!(decision.result_kind, "status.degraded");
    assert_eq!(decision.slot_id.as_deref(), Some("slot-01"));
    let calls = runtime.calls.lock().expect("probe calls");
    assert_eq!(calls.get("slot-01"), Some(&2));
    assert_eq!(calls.get("slot-02"), Some(&1));
}

#[test]
fn state_invalid_precedes_lock_contention_on_the_json_surface() {
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-cli-status-precedence-{}-{}",
        std::process::id(),
        gpt_webai_lifecycle::config::now_ms()
    ));
    create_private_test_dir(&root);
    create_private_test_dir(&root.join("sessions"));
    let invalid_session = root.join("sessions/bad.json");
    fs::write(&invalid_session, b"{}\n").expect("invalid session fixture");
    fs::set_permissions(&invalid_session, fs::Permissions::from_mode(0o600)).expect("session mode");
    create_private_test_dir(&root.join("journal"));
    create_private_test_dir(&root.join("journal/locks"));
    create_private_test_dir(&root.join("journal/locks/mutation.lock"));

    let output = Command::new(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
        .args(["status", "--json"])
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("GPT_WEBAI_SLOT_MODE", "fake")
        .output()
        .expect("status command");
    assert_eq!(output.status.code(), Some(70));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["resultKind"], "status.state_invalid");
    assert_eq!(value["reason"], "journal.immutable_collision");

    fs::remove_dir_all(root).expect("remove status fixture");
}

#[test]
fn guard_metadata_failure_emits_a_state_invalid_envelope() {
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-cli-status-guard-{}-{}",
        std::process::id(),
        gpt_webai_lifecycle::config::now_ms()
    ));
    create_private_test_dir(&root);
    create_private_test_dir(&root.join("journal"));
    fs::create_dir(root.join("journal/locks")).expect("unsafe locks fixture");
    fs::set_permissions(
        root.join("journal/locks"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("unsafe locks mode");

    let output = Command::new(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
        .args(["status", "--json"])
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("GPT_WEBAI_SLOT_MODE", "fake")
        .output()
        .expect("status command");
    assert_eq!(output.status.code(), Some(70));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["resultKind"], "status.state_invalid");
    assert_eq!(value["reason"], "journal.immutable_collision");

    fs::remove_dir_all(root).expect("remove status fixture");
}

#[derive(Default)]
struct RetryRuntimeProbe {
    calls: Mutex<BTreeMap<String, usize>>,
}

impl gpt_webai_lifecycle::runtime::RuntimeProbe for RetryRuntimeProbe {
    fn observe(&self, slot: &SlotConfig) -> RuntimeObservation {
        let mut calls = self.calls.lock().expect("probe calls");
        let count = calls.entry(slot.slot_id.0.clone()).or_default();
        *count += 1;
        let provider_readiness = if slot.slot_id.0 == "slot-01" {
            if *count == 1 {
                ProviderReadiness::Unreachable
            } else {
                ProviderReadiness::Unknown
            }
        } else {
            ProviderReadiness::NotChecked
        };
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: None,
            provider_readiness,
        }
    }
}

fn create_private_test_dir(path: &std::path::Path) {
    fs::create_dir(path).expect("private test directory");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .expect("private test directory mode");
}

fn config(slot_count: u8) -> SupervisorConfig {
    SupervisorConfig {
        state_root: PathBuf::from("/tmp/pr72-cli-status-unit"),
        slot_count,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "fake".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    }
}

fn slot_record(slot_id: &str, health_status: &str) -> SlotRecord {
    SlotRecord {
        slot_id: slot_id.to_string(),
        cohort: gpt_webai_lifecycle::allocator::cohort_of(slot_id)
            .expect("cohort")
            .to_string(),
        health_status: health_status.to_string(),
        docker_status: "running".to_string(),
        allocatable: false,
        cooldown_until_ms: None,
        standby: false,
        last_event_id: event_id('a'),
    }
}

fn active_lease(slot_id: &str, generation: u16) -> CasRecord {
    CasRecord {
        id: format!("lease-{slot_id}"),
        kind: "slot_lease".to_string(),
        subject_id: slot_id.to_string(),
        owner: Writer {
            host_id: "host_0123456789abcdef0123456789abcdef".to_string(),
            process_id: 1,
            process_start_ms: 1,
            writer_id: "writer_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        },
        generation,
        renewal_revision: 1,
        fencing_token_sha256: Some(format!("sha256:{}", "a".repeat(64))),
        granted_at_ms: 1,
        renew_at_ms: 100_001,
        expires_at_ms: u64::MAX,
        status: "active".to_string(),
        released_at_ms: None,
        release_event_id: None,
        last_event_id: event_id('b'),
    }
}

fn event_id(value: char) -> String {
    format!("evt_{}", value.to_string().repeat(64))
}
