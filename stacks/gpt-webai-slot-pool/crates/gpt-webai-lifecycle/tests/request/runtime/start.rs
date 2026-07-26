use std::time::Duration;
use std::{fs, os::unix::fs::PermissionsExt};

use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::{run_provider_round_trip, RequestRunInput};
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use serde_json::json;

use crate::support::{standby_exited_runtime, FakeRun, InputSpec};

#[test]
fn standby_exited_slot_starts_runtime_before_provider_and_stops_on_release() {
    let fixture = FakeRun::new("start-runtime-success");
    write_standby_state(&fixture);
    let docker_log = fixture.path().join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);
    let mut input = input(&fixture, "sid-start-runtime");
    input.runtime_start_mode = RuntimeStartMode::StartRuntime {
        docker_bin: docker_bin.clone(),
        timeout: Duration::from_secs(2),
    };
    input.runtime_release_mode = RuntimeReleaseMode::StopRuntime {
        docker_bin,
        timeout: Duration::from_secs(2),
    };

    let output = run_provider_round_trip(input, &standby_exited_runtime());

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert!(output.runtime_started);
    assert!(output.runtime_owned);
    assert!(output.runtime_stopped);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker args"),
        "start\ngpt-webai-slot-01\nstop\ngpt-webai-slot-01\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("slots").join("slot-01.state"))
            .expect("slot state"),
        "status=standby\n"
    );
}

#[test]
fn runtime_start_failure_releases_lock_without_provider_invocation() {
    let fixture = FakeRun::new("start-runtime-failure");
    write_standby_state(&fixture);
    let docker_log = fixture.path().join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 9);
    let mut input = input(&fixture, "sid-start-failed");
    input.runtime_start_mode = RuntimeStartMode::StartRuntime {
        docker_bin,
        timeout: Duration::from_secs(2),
    };

    let output = run_provider_round_trip(input, &standby_exited_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("runtime.start_failed"));
    assert!(!output.runtime_started);
    assert!(!output.runtime_owned);
    assert!(output.lock_released);
    assert!(!output.runtime_stopped);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker args"),
        "start\ngpt-webai-slot-01\n"
    );
    assert!(!fixture.args_log.exists());
}

fn input(fixture: &FakeRun, session_id: &str) -> RequestRunInput {
    let send_json = fixture.write_json("send.json", sent(session_id));
    let poll_json = fixture.write_json("poll.json", done(session_id));
    fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    })
}

fn write_standby_state(fixture: &FakeRun) {
    let slots_dir = fixture.path().join("slots");
    fs::create_dir_all(&slots_dir).expect("slots dir");
    fs::set_permissions(&slots_dir, fs::Permissions::from_mode(0o700)).expect("private slots dir");
    let state_path = slots_dir.join("slot-01.state");
    fs::write(&state_path, "status=standby\n").expect("slot state");
    fs::set_permissions(state_path, fs::Permissions::from_mode(0o600)).expect("private slot state");
}

fn sent(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "sent",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "turnEvidence": {
            "activeTurn": true,
            "userTurnId": format!("turn_{}", "1".repeat(64)),
            "assistantTurnId": format!("turn_{}", "2".repeat(64))
        }
    })
}

fn done(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    })
}
