use std::fs;

use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::artifact_expectation::ArtifactExpectation;
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::sessions::read_session_record;

use crate::request::artifacts::fixtures::{
    done_no_artifacts, done_with_candidate, download_done, download_recovery_failed, sent,
};
use crate::support::{ready_runtime, FakeRun, InputSpec};

#[test]
fn downloads_terminal_artifacts_before_releasing_slot() {
    let fixture = FakeRun::new("artifact-success");
    let send_json = fixture.write_json("send.json", sent("sid-artifact"));
    let poll_json = fixture.write_json("poll.json", done_with_candidate("sid-artifact"));
    let download_json = fixture.write_json("download.json", download_done("sid-artifact", 1));

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: Some(download_json),
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.download_status.as_deref(), Some("done"));
    assert_eq!(output.artifact_candidates, 1);
    assert_eq!(output.artifacts, 1);
    assert_eq!(
        output.answer_text_len,
        Some("final answer with artifact".len())
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-artifact").expect("session record");
    assert_eq!(session.session_id, "sid-artifact");
    assert!(session.updated_at_ms >= session.created_at_ms);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    let lines = args.lines().map(str::trim).collect::<Vec<_>>();
    assert_eq!(
        lines,
        vec![
            "send --prompt-file ".to_string()
                + fixture.prompt_file.to_str().expect("prompt path")
                + " --model pro --effort extended",
            "poll --session sid-artifact --timeout 30 --artifact-expectation optional".to_string(),
            "download --session sid-artifact --artifact-expectation optional".to_string(),
        ]
    );
    let artifact_dir = fixture
        .path()
        .join("requests")
        .join("run-a")
        .join("artifacts");
    assert!(artifact_dir.join("provider-download.json").exists());
    let manifest = fs::read_to_string(artifact_dir.join("artifact-objects.json"))
        .expect("artifact objects manifest");
    let value = serde_json::from_str::<serde_json::Value>(&manifest).expect("manifest json");
    assert_eq!(value["schema"], "gpt-webai.artifact-objects.v1");
    assert_eq!(value["sessionId"], "sid-artifact");
    assert_eq!(value["artifacts"][0]["buttonText"], "pr72-artifact.zip");
}

#[test]
fn fails_closed_when_artifact_candidates_download_zero_files() {
    let fixture = FakeRun::new("artifact-zero");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-zero"));
    let poll_json = fixture.write_json("poll.json", done_with_candidate("sid-artifact-zero"));
    let download_json = fixture.write_json("download.json", download_done("sid-artifact-zero", 0));

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: Some(download_json),
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("artifact.recovery_failed"));
    assert_eq!(output.download_status.as_deref(), Some("done"));
    assert_eq!(output.artifact_candidates, 1);
    assert_eq!(output.artifacts, 0);
    assert_eq!(
        output.answer_text_len,
        Some("final answer with artifact".len())
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-artifact-zero").expect("session record");
    assert_eq!(session.session_id, "sid-artifact-zero");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn fails_closed_when_download_session_mismatches_terminal_session() {
    let fixture = FakeRun::new("artifact-mismatch");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-mismatch"));
    let poll_json = fixture.write_json("poll.json", done_with_candidate("sid-artifact-mismatch"));
    let download_json = fixture.write_json("download.json", download_done("sid-other", 1));

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: Some(download_json),
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("artifact.recovery_failed"));
    assert_eq!(output.download_status.as_deref(), Some("done"));
    assert_eq!(output.artifacts, 1);
    assert_eq!(
        output.answer_text_len,
        Some("final answer with artifact".len())
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session =
        read_session_record(fixture.path(), "sid-artifact-mismatch").expect("session record");
    assert_eq!(session.session_id, "sid-artifact-mismatch");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn fails_closed_when_provider_reports_artifact_recovery_failed() {
    let fixture = FakeRun::new("artifact-provider-failed");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-provider-failed"));
    let poll_json = fixture.write_json(
        "poll.json",
        done_with_candidate("sid-artifact-provider-failed"),
    );
    let download_json = fixture.write_json(
        "download.json",
        download_recovery_failed("sid-artifact-provider-failed"),
    );

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: Some(download_json),
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("artifact.recovery_failed"));
    assert_eq!(
        output.download_status.as_deref(),
        Some("artifact.recovery_failed")
    );
    assert_eq!(output.artifact_candidates, 1);
    assert_eq!(output.artifacts, 0);
    assert_eq!(
        output.answer_text_len,
        Some("final answer with artifact".len())
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
}

#[test]
fn accepts_no_artifact_terminal_answer_when_artifacts_optional() {
    let fixture = FakeRun::new("artifact-optional-none");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-optional-none"));
    let poll_json =
        fixture.write_json("poll.json", done_no_artifacts("sid-artifact-optional-none"));

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: None,
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert_eq!(output.artifact_candidates, 0);
    assert_eq!(output.artifacts, 0);
    let session =
        read_session_record(fixture.path(), "sid-artifact-optional-none").expect("session record");
    assert_eq!(session.session_id, "sid-artifact-optional-none");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn fails_controls_absent_when_artifacts_required_but_provider_returns_none() {
    let fixture = FakeRun::new("artifact-required-none");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-required-none"));
    let poll_json =
        fixture.write_json("poll.json", done_no_artifacts("sid-artifact-required-none"));
    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.artifact_expectation = ArtifactExpectation::Required;

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("artifact.controls_absent"));
    assert_eq!(output.artifact_candidates, 0);
    assert_eq!(output.artifacts, 0);
    assert_eq!(
        output.answer_text_len,
        Some("final answer without artifact".len())
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session =
        read_session_record(fixture.path(), "sid-artifact-required-none").expect("session record");
    assert_eq!(session.session_id, "sid-artifact-required-none");
    assert!(session.updated_at_ms >= session.created_at_ms);
}
