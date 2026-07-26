use std::fs;
use std::process::Command;

use gpt_webai_lifecycle::contracts::events::EventType;
use gpt_webai_lifecycle::journal::EventStore;
use gpt_webai_lifecycle::records::{holder_count, lock_count};

use super::fixtures::{binary, done_resume, stdout_json, Fixture};
use crate::run_fixture::{
    binary as run_binary, stdout_json as run_stdout_json, Fixture as RunFixture,
};

#[test]
fn cli_resume_fake_provider_uses_persisted_session_slot() {
    let fixture = RunFixture::new("resume-r13");
    let docker = fixture.write_fake_docker();

    let started = Command::new(run_binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS", "1000")
        .args([
            "run",
            "--json",
            "--docker-slot-provider",
            "--live-send",
            "--require-visual-gate",
            "--docker-bin",
        ])
        .arg(&docker)
        .args([
            "--prompt-file",
            fixture.prompt.to_str().expect("prompt path"),
            "--file",
            fixture.upload_one.to_str().expect("upload one path"),
            "--file",
            fixture.upload_two.to_str().expect("upload two path"),
            "--request-id",
            "request-cli-resume",
            "--run-id",
            "run-cli-resume",
            "--fencing-token",
            "fixture-fence",
            "--model",
            "pro",
            "--effort",
            "standard",
            "--artifact-expectation",
            "optional",
            "--provider-timeout-ms",
            "500000",
            "--runtime-stop-timeout-ms",
            "1000",
            "--runtime-start-timeout-ms",
            "1000",
        ])
        .output()
        .expect("run initial R13 request");
    assert!(
        started.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    let started_value = run_stdout_json(&started.stdout);
    let session_id = started_value["sessionId"]
        .as_str()
        .expect("run session id")
        .to_string();
    assert_eq!(started_value["resultKind"], "run.terminal_optional_zero");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS", "1000")
        .args(["resume", "--json", "--session"])
        .arg(&session_id)
        .args([
            "--fencing-token",
            "fixture-fence",
            "--docker-slot-provider",
            "--docker-bin",
        ])
        .arg(&docker)
        .args([
            "--provider-timeout-ms",
            "500000",
            "--runtime-stop-timeout-ms",
            "1000",
            "--runtime-start-timeout-ms",
            "1000",
        ])
        .output()
        .expect("run cli");

    let journal_debug = EventStore::new(&fixture.root)
        .load_all()
        .unwrap_or_default()
        .into_iter()
        .map(|event| {
            format!(
                "{:?} {} {:?} {:?}",
                event.event_type, event.event_id, event.aggregate, event.predecessor_event_id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.status.success(),
        "stdout={} stderr={} journal={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        journal_debug,
    );
    let value = stdout_json(&output.stdout);
    assert_eq!(value["ok"], true);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "resume");
    assert_eq!(value["resultKind"], "resume.terminal_optional_zero");
    assert_eq!(value["status"], "resume.terminal_optional_zero");
    assert_eq!(value["sessionId"], session_id);
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(value["cohort"], "cohort-a");
    assert_eq!(value["answerText"], "final answer");
    assert_eq!(value["answerSizeBytes"], 12);
    assert!(value["answerPath"].as_str().is_some());
    assert!(value["answerSha256"]
        .as_str()
        .is_some_and(|item| item.starts_with("sha256:")));
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);

    assert_eq!(
        fs::read_to_string(&fixture.provider_operations).expect("provider operations"),
        "status\ncapture.root\nensure-model\nupload-only\nsend-click\nsession-rebind\npoll\nartifact-discover\nstatus\nsession-rebind\npoll\nartifact-discover\n"
    );

    let mut events = EventStore::new(&fixture.root)
        .load_all()
        .expect("load R13 journal");
    events.sort_by_key(|event| (event.created_at_ms, event.event_id.clone()));
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(ordered_subsequence(
        &event_types,
        &[
            EventType::TurnStartConfirmed,
            EventType::SessionBindingEstablished,
            EventType::RunningProjected,
            EventType::SessionOperationClaimGranted,
            EventType::PersistedSessionLeaseGranted,
            EventType::SessionRuntimeOwnershipGranted,
            EventType::SessionRebindStarted,
            EventType::SessionRebound,
            EventType::SessionHydrated,
            EventType::PollStarted,
            EventType::AnswerTerminal,
            EventType::ArtifactControlsAbsent,
            EventType::ArtifactClaimCompleted,
            EventType::TerminalPersisted,
            EventType::OutputPublished,
            EventType::SessionOperationClaimReleased,
            EventType::SlotLeaseReleased,
            EventType::RuntimeOwnershipReleased,
            EventType::ReleaseFinalized,
        ],
    ));
}

fn ordered_subsequence(actual: &[EventType], expected: &[EventType]) -> bool {
    let mut cursor = 0;
    for item in actual {
        if expected.get(cursor) == Some(item) {
            cursor += 1;
        }
    }
    cursor == expected.len()
}

#[test]
fn cli_resume_rejects_retired_kind_surface_without_provider_mutation() {
    let fixture = Fixture::new("legacy");
    fixture.write_session("sid-cli-resume-legacy", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("sid-cli-resume-legacy"));

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .args([
            "resume",
            "--kind",
            "pro",
            "--session",
            "sid-cli-resume-legacy",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run legacy cli");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!args_file.exists());
}

#[test]
fn cli_resume_missing_session_returns_json_failure_without_provider_call() {
    let fixture = Fixture::new("missing");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("missing"));

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .args([
            "resume",
            "--json",
            "--session",
            "missing",
            "--fencing-token",
            "fixture-fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run cli");

    assert_eq!(
        output.status.code(),
        Some(70),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output.stdout);
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "resume");
    assert_eq!(value["resultKind"], "resume.unknown_session");
    assert_eq!(value["reason"], "session.missing");
    assert_eq!(value["sessionId"], "missing");
    assert!(!args_file.exists());
}

#[test]
fn cli_resume_missing_durable_request_projection_is_a_pre_acquisition_failure() {
    let fixture = Fixture::new("request-projection-missing");
    fixture.write_session("sid-cli-resume-unbound", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("sid-cli-resume-unbound"));

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("XDG_STATE_HOME", fixture.root.join("xdg-state"))
        .env("HOME", fixture.root.join("home"))
        .args([
            "resume",
            "--json",
            "--session",
            "sid-cli-resume-unbound",
            "--fencing-token",
            "fixture-fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run unbound resume");

    assert_eq!(output.status.code(), Some(70));
    assert!(output.stderr.is_empty());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["resultKind"], "resume.request_binding_missing");
    assert_eq!(value["reason"], "session.request_binding_missing");
    assert_eq!(value["eventIds"], serde_json::json!([]));
    assert!(!args_file.exists());
}
