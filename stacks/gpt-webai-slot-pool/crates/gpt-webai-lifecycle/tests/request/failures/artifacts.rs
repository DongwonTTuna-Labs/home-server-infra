use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use serde_json::json;

use super::fixtures::sent;

#[test]
fn artifact_timeout_poll_persists_raw_evidence_before_release() {
    let fixture = FakeRun::new("artifact-timeout-poll");
    let send_json = fixture.write_json("send.json", sent("sid-artifact-timeout"));
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "artifact.download_timeout",
            "reason": "artifact.download_timeout",
            "sessionId": "sid-artifact-timeout",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-artifact-timeout",
            "artifacts": [],
            "artifactCandidates": [{
                "sessionId": "sid-artifact-timeout",
                "buttonText": "artifact-timeout.zip",
                "buttonTextSha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "turnScope": "current-assistant-turn",
                "clickedElement": {
                    "role": "button",
                    "tag": "button"
                },
                "artifact": {
                    "status": "failed",
                    "reason": "artifact.download_timeout"
                }
            }],
            "warnings": [{
                "reason": "artifact.download_timeout"
            }],
            "downloadCandidateCount": 1
        }),
    );

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: None,
            files: Vec::new(),
        }),
        &ready_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("answer.unconfirmed"));
    assert_eq!(
        output.poll_status.as_deref(),
        Some("artifact.download_timeout")
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let artifact_dir = fixture
        .path()
        .join("requests")
        .join("run-a")
        .join("artifacts");
    assert!(artifact_dir.join("provider-poll.json").exists());
    let manifest = std::fs::read_to_string(artifact_dir.join("poll-artifact-objects.json"))
        .expect("poll artifact manifest");
    let value = serde_json::from_str::<serde_json::Value>(&manifest).expect("manifest json");
    assert_eq!(value["schema"], "gpt-webai.artifact-objects.v1");
    assert_eq!(
        value["artifactCandidates"][0]["buttonText"],
        "artifact-timeout.zip"
    );
    assert_eq!(value["warnings"][0]["reason"], "artifact.download_timeout");
}
