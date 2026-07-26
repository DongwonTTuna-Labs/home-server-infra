use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use gpt_webai_lifecycle::show::{show_session, ProviderLimits, ShowInput};

#[test]
fn show_session_fails_closed_before_provider_when_session_record_is_missing() {
    let root = temp_state_root("show-missing");
    let outcome = show_session(ShowInput {
        config: config(root.clone()),
        operation_id: "show-missing-operation".to_string(),
        session_id: "missing-session".to_string(),
        fencing_token: "fixture-fence".to_string(),
        provider_execution: ProviderExecution::Host(HostProviderExecution {
            provider_bin: root.join("must-not-run"),
            args_prefix: Vec::new(),
            env: Vec::new(),
        }),
        runtime_start_mode: RuntimeStartMode::Disabled,
        runtime_release_mode: RuntimeReleaseMode::LockOnly,
        provider_limits: ProviderLimits {
            timeout: Duration::from_secs(1),
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 262_144,
        },
    })
    .expect("closed unknown-session outcome");

    assert_eq!(outcome.exit_code, 70);
    assert!(!outcome.envelope.ok);
    assert_eq!(outcome.envelope.schema, "gpt-webai.lifecycle.r13.v1");
    assert_eq!(outcome.envelope.result_kind, "show.unknown_session");
    assert_eq!(outcome.envelope.reason.as_deref(), Some("session.missing"));
    assert_eq!(
        outcome.envelope.session_id.as_deref(),
        Some("missing-session")
    );
    assert!(outcome.envelope.event_ids.is_empty());
    assert!(outcome.envelope.receipt_ids.is_empty());
}

fn config(state_root: PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        state_root,
        slot_count: 10,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 1_000,
    }
}

fn temp_state_root(name: &str) -> PathBuf {
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
