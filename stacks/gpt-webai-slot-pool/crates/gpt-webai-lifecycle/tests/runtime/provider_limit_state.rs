use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::records::read_key_value_file;
use gpt_webai_lifecycle::runtime::control::write_slot_status;

#[test]
fn provider_limit_state_records_recheck_metadata() {
    let state_root = temp_state_root("provider-limit-state");
    let config = SupervisorConfig {
        state_root: state_root.clone(),
        slot_count: 1,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };

    write_slot_status(&config, "slot-01", "provider.limit").expect("write state");

    let values =
        read_key_value_file(&state_root.join("slots").join("slot-01.state")).expect("read state");
    assert_eq!(
        values.get("status").map(String::as_str),
        Some("provider.limit")
    );
    assert!(values.contains_key("provider_limit_observed_at_ms"));
    assert!(values.contains_key("provider_limit_next_retry_at_ms"));
    let observed_at = parse_ms(&values, "provider_limit_observed_at_ms");
    let next_retry_at = parse_ms(&values, "provider_limit_next_retry_at_ms");
    assert_eq!(next_retry_at.saturating_sub(observed_at), 180_000);
    let _ = fs::remove_dir_all(state_root);
}

fn parse_ms(values: &std::collections::BTreeMap<String, String>, key: &str) -> u64 {
    values
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .expect(key)
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
