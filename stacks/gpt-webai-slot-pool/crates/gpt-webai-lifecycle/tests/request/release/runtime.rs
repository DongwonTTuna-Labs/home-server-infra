use std::time::Duration;

use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::{run_provider_round_trip, RequestRunInput};
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use gpt_webai_lifecycle::sessions::read_session_record;
use serde_json::json;

use crate::support::{ready_runtime, FakeRun, InputSpec};

#[test]
fn terminal_run_stops_runtime_before_releasing_lock() {
    let fixture = FakeRun::new("release-runtime-success");
    let docker_log = fixture.path().join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);
    let mut input = input(&fixture, "sid-release-runtime");
    input.runtime_start_mode = RuntimeStartMode::StartRuntime {
        docker_bin: docker_bin.clone(),
        timeout: Duration::from_secs(2),
    };
    input.runtime_release_mode = RuntimeReleaseMode::StopRuntime {
        docker_bin,
        timeout: Duration::from_secs(2),
    };

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert!(output.runtime_stopped);
    assert!(output.slot_state_written);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker args"),
        "stop\ngpt-webai-slot-01\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("slots").join("slot-01.state"))
            .expect("slot state"),
        "status=standby\n"
    );
    let session = read_session_record(fixture.path(), "sid-release-runtime").expect("session");
    assert_eq!(session.session_id, "sid-release-runtime");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn runtime_stop_failure_preserves_lock_and_marks_session_release_failed() {
    let fixture = FakeRun::new("release-runtime-failure");
    let docker_log = fixture.path().join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 9);
    let mut input = input(&fixture, "sid-release-runtime-failure");
    input.runtime_start_mode = RuntimeStartMode::StartRuntime {
        docker_bin: docker_bin.clone(),
        timeout: Duration::from_secs(2),
    };
    input.runtime_release_mode = RuntimeReleaseMode::StopRuntime {
        docker_bin,
        timeout: Duration::from_secs(2),
    };

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.status, "release_failed");
    assert_eq!(output.reason.as_deref(), Some("runtime.stop_failed"));
    assert!(!output.runtime_stopped);
    assert!(!output.slot_state_written);
    assert!(!output.lock_released);
    assert_eq!(holder_count(fixture.path()), 1);
    assert_eq!(lock_count(fixture.path()), 1);
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker args"),
        "stop\ngpt-webai-slot-01\n"
    );
    assert!(!fixture.path().join("slots").join("slot-01.state").exists());
    let session =
        read_session_record(fixture.path(), "sid-release-runtime-failure").expect("session");
    assert_eq!(session.session_id, "sid-release-runtime-failure");
    assert_eq!(session.slot_id, "slot-01");
    assert_eq!(session.cohort, "cohort-a");
    assert!(session.updated_at_ms >= session.created_at_ms);
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
