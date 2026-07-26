use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use super::fixtures::{binary, stdout_json, Fixture};

#[test]
fn cli_download_fake_provider_uses_persisted_session_slot() {
    let fixture = Fixture::new("done");
    fixture.write_session("sid-cli-download", "slot-06", "cohort-b");
    let provider = fixture.provider_bin();
    let script = fixture.write_provider_script(&["status", "session-rebind", "artifact-discover"]);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_FAKE_SCRIPT", &script)
        .args([
            "download",
            "--json",
            "--session",
            "sid-cli-download",
            "--fencing-token",
            "fixture-fence",
            "--artifact-expectation",
            "optional",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run cli");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output.stdout);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "download");
    assert_eq!(value["resultKind"], "download.optional_zero");
    assert_eq!(value["status"], "download.optional_zero");
    assert_eq!(value["sessionId"], "sid-cli-download");
    assert_eq!(value["slotId"], "slot-06");
    assert_eq!(value["cohort"], "cohort-b");
    assert_eq!(
        value["conversationUrl"],
        "https://chatgpt.com/c/sid-cli-download"
    );
    assert_eq!(value["artifactClaims"][0]["expectation"], "optional");
    assert_eq!(value["artifactClaims"][0]["status"], "completed");
    assert_eq!(fixture.provider_invocation_count(&script), 3);
    let requests = fixture.provider_requests();
    assert!(requests
        .iter()
        .all(|request| request["identity"]["slotId"] == "slot-06"));
    let discover = requests
        .iter()
        .find(|request| request["operation"] == "artifact-discover")
        .expect("artifact discover request");
    assert_eq!(discover["operationData"]["expectation"], "optional");
}

#[test]
fn cli_download_forwards_explicit_artifact_expectation() {
    let fixture = Fixture::new("explicit-artifact-expectation");
    fixture.write_session("sid-cli-download-required", "slot-06", "cohort-b");
    let provider = fixture.provider_bin();
    let script = fixture.write_provider_script(&["status", "session-rebind", "artifact-discover"]);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_FAKE_SCRIPT", &script)
        .args([
            "download",
            "--json",
            "--session",
            "sid-cli-download-required",
            "--fencing-token",
            "fixture-fence",
            "--artifact-expectation",
            "required",
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
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "download");
    assert_eq!(value["resultKind"], "download.controls_absent_required");
    assert_eq!(value["status"], "download.controls_absent_required");
    assert_eq!(value["reason"], "artifact.required_zero");
    assert_eq!(value["sessionId"], "sid-cli-download-required");
    assert_eq!(value["artifactClaims"][0]["expectation"], "required");
    assert_eq!(value["artifactClaims"][0]["status"], "failed");
    assert_eq!(fixture.provider_invocation_count(&script), 3);
    let requests = fixture.provider_requests();
    let discover = requests
        .iter()
        .find(|request| request["operation"] == "artifact-discover")
        .expect("artifact discover request");
    assert_eq!(discover["operationData"]["expectation"], "required");
}

#[test]
fn cli_download_rejects_retired_kind_surface_without_provider_mutation() {
    let fixture = Fixture::new("legacy");
    fixture.write_session("sid-cli-download-legacy", "slot-06", "cohort-b");
    let provider = fixture.provider_bin();
    let script = fixture.write_provider_script(&[]);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .args([
            "download",
            "--kind",
            "pro",
            "--session",
            "sid-cli-download-legacy",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run legacy cli");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(fixture.provider_invocation_count(&script), 0);
}

#[test]
fn cli_download_missing_session_returns_json_failure_without_provider_call() {
    let fixture = Fixture::new("missing");
    let provider = fixture.provider_bin();
    let script = fixture.write_provider_script(&[]);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_FAKE_SCRIPT", &script)
        .args([
            "download",
            "--json",
            "--session",
            "missing",
            "--fencing-token",
            "fixture-fence",
            "--artifact-expectation",
            "optional",
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
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "download");
    assert_eq!(value["resultKind"], "download.unknown_session");
    assert_eq!(value["reason"], "session.missing");
    assert_eq!(value["sessionId"], "missing");
    assert_eq!(fixture.provider_invocation_count(&script), 0);
}

#[test]
fn cli_download_reports_first_mutation_lock_contention_as_an_r13_envelope() {
    let fixture = Fixture::new("lock-contended");
    fixture.write_session("sid-cli-download-lock", "slot-06", "cohort-b");
    for relative in ["journal", "journal/locks", "journal/locks/mutation.lock"] {
        let path = fixture.root.join(relative);
        fs::create_dir(&path).expect("private contended lock component");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("private contended lock mode");
    }
    let provider = fixture.provider_bin();
    let script = fixture.write_provider_script(&[]);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_FAKE_SCRIPT", &script)
        .env("XDG_STATE_HOME", fixture.root.join("xdg-state"))
        .env("HOME", fixture.root.join("home"))
        .args([
            "download",
            "--json",
            "--session",
            "sid-cli-download-lock",
            "--fencing-token",
            "fixture-fence",
            "--artifact-expectation",
            "optional",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run lock-contended download");

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stderr.is_empty());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["resultKind"], "download.lock_contended");
    assert_eq!(value["reason"], "lock.contended");
    assert_eq!(value["eventIds"], serde_json::json!([]));
    assert_eq!(fixture.provider_invocation_count(&script), 0);
}
