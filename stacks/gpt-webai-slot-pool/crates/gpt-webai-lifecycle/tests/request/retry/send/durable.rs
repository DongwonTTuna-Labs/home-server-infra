use std::fs;
use std::time::Duration;

use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::sessions::read_session_record;

use super::fixtures::{command_count, done, write_durable_recovery_provider};

#[test]
fn durable_send_start_evidence_recovers_provider_json_failure_without_duplicate_send() {
    let mut fixture = FakeRun::new("send-recovery-durable-confirmed");
    let poll_json = fixture.write_json("poll.json", done("sid-durable-recovered"));
    let unused_send = fixture.write_file("unused-send.json", "{}");
    fixture.provider = write_durable_recovery_provider(
        fixture.path(),
        &fixture.args_log,
        "sid-durable-recovered",
        true,
    );
    let artifacts_dir = fixture.path().join("artifacts").join("durable-confirmed");

    let mut input = fixture.input(InputSpec {
        send_json: unused_send,
        poll_json: poll_json.clone(),
        download_json: None,
        files: Vec::new(),
    });
    input.provider_execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: fixture.provider.clone(),
        args_prefix: Vec::new(),
        env: vec![
            (
                "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
                artifacts_dir.display().to_string(),
            ),
            (
                "FAKE_PROVIDER_POLL_JSON".to_string(),
                poll_json.display().to_string(),
            ),
            (
                "FAKE_PROVIDER_ARGS_LOG".to_string(),
                fixture.args_log.display().to_string(),
            ),
        ],
    });
    input.send_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(output.reason, None);
    assert_eq!(output.session_id.as_deref(), Some("sid-durable-recovered"));
    assert_eq!(
        output.conversation_url.as_deref(),
        Some("https://chatgpt.com/c/sid-durable-recovered")
    );
    assert_eq!(output.send_status.as_deref(), Some("sent"));
    assert_eq!(output.send_attempts, 1);
    assert!(output.send_retry_delays_ms.is_empty());
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 1);
    let session =
        read_session_record(fixture.path(), "sid-durable-recovered").expect("session record");
    assert_eq!(session.session_id, "sid-durable-recovered");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn durable_send_start_without_turn_evidence_is_unconfirmed_and_not_retried() {
    let mut fixture = FakeRun::new("send-recovery-durable-unconfirmed");
    let poll_json = fixture.write_file("unused-poll.json", "{}");
    let unused_send = fixture.write_file("unused-send.json", "{}");
    fixture.provider = write_durable_recovery_provider(
        fixture.path(),
        &fixture.args_log,
        "sid-durable-unconfirmed",
        false,
    );
    let artifacts_dir = fixture.path().join("artifacts").join("durable-unconfirmed");

    let mut input = fixture.input(InputSpec {
        send_json: unused_send,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.provider_execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: fixture.provider.clone(),
        args_prefix: Vec::new(),
        env: vec![
            (
                "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
                artifacts_dir.display().to_string(),
            ),
            (
                "FAKE_PROVIDER_ARGS_LOG".to_string(),
                fixture.args_log.display().to_string(),
            ),
        ],
    });
    input.send_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("session.start_unconfirmed"));
    assert_eq!(
        output.session_id.as_deref(),
        Some("sid-durable-unconfirmed")
    );
    assert_eq!(
        output.conversation_url.as_deref(),
        Some("https://chatgpt.com/c/sid-durable-unconfirmed")
    );
    assert_eq!(
        output.send_status.as_deref(),
        Some("session.start_unconfirmed")
    );
    assert_eq!(output.send_attempts, 1);
    assert!(output.send_retry_delays_ms.is_empty());
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 0);
}
