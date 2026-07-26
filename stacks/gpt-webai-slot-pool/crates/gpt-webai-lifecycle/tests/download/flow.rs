use std::fs;

use gpt_webai_lifecycle::download::download_session;
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::artifact_expectation::ArtifactExpectation;
use serde_json::json;

use super::fixtures::Fixture;

#[test]
fn fake_download_uses_pinned_session_slot_without_locking_or_allocating() {
    let fixture = Fixture::new("done");
    fixture.write_session("sid-download", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_download("sid-download"));

    let output = download_session(fixture.input(provider, "sid-download"));

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert_eq!(output.session_id.as_deref(), Some("sid-download"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-06"));
    assert_eq!(output.account_group.as_deref(), Some("cohort-b"));
    assert_eq!(output.provider_status.as_deref(), Some("done"));
    assert_eq!(output.artifacts, 0);
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
    assert_eq!(
        fs::read_to_string(args_file).expect("args"),
        "download\n--session\nsid-download\n"
    );
}

#[test]
fn fake_download_forwards_explicit_artifact_expectation() {
    let fixture = Fixture::new("explicit-artifact-expectation");
    fixture.write_session("sid-download-required", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_download("sid-download-required"));
    let mut input = fixture.input(provider, "sid-download-required");
    input.artifact_expectation = Some(ArtifactExpectation::Required);

    let output = download_session(input);

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert_eq!(output.session_id.as_deref(), Some("sid-download-required"));
    assert_eq!(
        fs::read_to_string(args_file).expect("args"),
        "download\n--session\nsid-download-required\n--artifact-expectation\nrequired\n"
    );
}

#[test]
fn fake_download_persists_provider_raw_and_artifact_object_manifest() {
    let fixture = Fixture::new("artifact-manifest");
    fixture.write_session("sid-download-artifact", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(
        &args_file,
        done_download_with_artifact("sid-download-artifact"),
    );

    let output = download_session(fixture.input(provider, "sid-download-artifact"));

    assert!(output.ok);
    assert_eq!(output.artifacts, 1);
    assert_eq!(output.artifact_candidates, 1);
    let artifact_dir = fixture
        .root
        .join("requests")
        .join("run-sid-download-artifact")
        .join("artifacts");
    let raw = fs::read_to_string(artifact_dir.join("provider-download.json"))
        .expect("raw provider download");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw).expect("raw json")["artifacts"][0]
            ["buttonText"],
        "bundle.zip"
    );
    let manifest =
        fs::read_to_string(artifact_dir.join("artifact-objects.json")).expect("artifact manifest");
    let value = serde_json::from_str::<serde_json::Value>(&manifest).expect("manifest json");
    assert_eq!(value["schema"], "gpt-webai.artifact-objects.v1");
    assert_eq!(value["sessionId"], "sid-download-artifact");
    assert_eq!(value["artifacts"][0]["buttonText"], "bundle.zip");
    assert_eq!(
        value["artifactCandidates"][0]["artifact"]["status"],
        "saved"
    );
}

#[test]
fn fake_download_fails_closed_before_provider_when_session_record_is_missing() {
    let fixture = Fixture::new("missing");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_download("missing"));

    let output = download_session(fixture.input(provider, "missing"));

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("session.record_missing"));
    assert_eq!(output.session_id.as_deref(), Some("missing"));
    assert!(!args_file.exists());
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
}

fn done_download(session_id: &str) -> String {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "artifacts": [],
        "artifactCandidates": []
    })
    .to_string()
}

fn done_download_with_artifact(session_id: &str) -> String {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "artifacts": [{
            "sessionId": session_id,
            "buttonText": "bundle.zip",
            "buttonTextSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "tag": "button",
                "role": "button"
            },
            "artifact": {
                "status": "saved",
                "hostPath": "/tmp/bundle.zip",
                "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "size": 12
            }
        }],
        "artifactCandidates": [{
            "sessionId": session_id,
            "buttonText": "bundle.zip",
            "buttonTextSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "tag": "button",
                "role": "button"
            },
            "artifact": {
                "status": "saved",
                "hostPath": "/tmp/bundle.zip",
                "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "size": 12
            }
        }],
        "warnings": [],
        "downloadCandidateCount": 1
    })
    .to_string()
}
