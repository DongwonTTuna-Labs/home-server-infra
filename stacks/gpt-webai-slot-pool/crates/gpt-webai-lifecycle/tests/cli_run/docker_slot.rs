use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use gpt_webai_lifecycle::contracts::events::EventType;
use gpt_webai_lifecycle::journal::EventStore;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::sessions::read_session_record;

use super::fixtures::{binary, stdout_json, Fixture};

#[test]
fn cli_run_lock_contention_emits_r13_before_modern_or_legacy_staging() {
    for surface in ["modern", "legacy"] {
        let fixture = Fixture::new(&format!("lock-{surface}"));
        let provider = fixture.write_fake_docker();
        for relative in ["journal", "journal/locks", "journal/locks/mutation.lock"] {
            let path = fixture.root.join(relative);
            fs::create_dir(&path).expect("private contended lock component");
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .expect("private contended lock mode");
        }

        let mut command = Command::new(binary());
        command
            .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
            .env("GPT_WEBAI_SLOT_MODE", "fake")
            .arg("run");
        if surface == "modern" {
            command
                .args([
                    "--json",
                    "--fake-runtime",
                    "--fake-provider",
                    "--provider-bin",
                ])
                .arg(&provider)
                .arg("--prompt-file")
                .arg(&fixture.prompt)
                .args([
                    "--request-id",
                    "request-lock-modern",
                    "--run-id",
                    "run-lock-modern",
                    "--fencing-token",
                    "token-lock-modern",
                    "--model",
                    "pro",
                    "--effort",
                    "standard",
                    "--artifact-expectation",
                    "optional",
                ]);
        } else {
            command.args([
                "--kind",
                "pro",
                "--prompt",
                "legacy prompt must not be staged",
                "--request-id",
                "request-lock-legacy",
                "--run-id",
                "run-lock-legacy",
                "--fencing-token",
                "token-lock-legacy",
            ]);
        }

        let output = command.output().expect("run lock-contention CLI");
        assert_eq!(
            output.status.code(),
            Some(75),
            "surface={surface} stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let value = stdout_json(&output.stdout);
        assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
        assert_eq!(value["command"], "run");
        assert_eq!(value["resultKind"], "run.lock_contended");
        assert_eq!(value["status"], "run.lock_contended");
        assert_eq!(value["ok"], false);
        assert_eq!(value["terminal"], true);
        assert_eq!(value["reason"], "lock.contended");
        assert_eq!(value["eventIds"], serde_json::json!([]));
        assert_eq!(value["receiptIds"], serde_json::json!([]));
        assert!(!fixture.root.join("requests/request-lock-modern").exists());
        assert!(!fixture
            .root
            .join("requests/legacy-inputs/run-lock-legacy")
            .exists());
    }
}

#[test]
fn cli_run_docker_slot_path_uses_visual_gate_send_poll_and_release() {
    let fixture = Fixture::new("docker-run");
    let docker = fixture.write_fake_docker();

    let output = Command::new(binary())
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
            "request-cli-docker",
            "--run-id",
            "run-cli-docker",
            "--fencing-token",
            "token-cli-docker",
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
        .expect("run cli");

    assert!(
        output.status.success(),
        "stderr={}\nprovider_operations={}\ndocker_log={}",
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.provider_operations).unwrap_or_default(),
        fs::read_to_string(&fixture.docker_log).unwrap_or_default()
    );
    let value = stdout_json(&output.stdout);
    assert_eq!(
        value["ok"],
        true,
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "run");
    assert_eq!(value["resultKind"], "run.terminal_optional_zero");
    assert_eq!(value["status"], "run.terminal_optional_zero");
    assert_eq!(value["terminal"], true);
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(value["sessionId"], "sid-cli-docker");
    assert_eq!(
        value["conversationUrl"],
        "https://chatgpt.com/c/sid-cli-docker"
    );
    assert_eq!(value["answerText"], "final answer");
    assert_eq!(value["answerSizeBytes"], 12);
    assert!(value["answerPath"].as_str().is_some());
    assert!(value["answerSha256"]
        .as_str()
        .is_some_and(|item| item.starts_with("sha256:")));
    assert!(value["eventIds"]
        .as_array()
        .is_some_and(|items| items.len() >= 30));
    assert!(value["receiptIds"]
        .as_array()
        .is_some_and(|items| items.len() >= 10));
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);

    let session = read_session_record(&fixture.root, "sid-cli-docker").expect("session");
    assert_eq!(session.session_id, "sid-cli-docker");
    assert_eq!(
        session.conversation_url,
        "https://chatgpt.com/c/sid-cli-docker"
    );
    assert!(session.updated_at_ms >= session.created_at_ms);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("WEB:"));

    let operations = fs::read_to_string(&fixture.provider_operations).expect("provider log");
    assert_eq!(
        operations,
        "status\ncapture.root\nensure-model\nupload-only\nsend-click\nsession-rebind\npoll\nartifact-discover\n"
    );
    let log = fs::read_to_string(&fixture.docker_log).expect("docker log");
    assert!(log.contains("compose -p gpt-webai-slot-pool up -d --force-recreate"));
    assert!(log.contains("node provider/chatgpt-playwright/cli.mjs --request-file"));
    assert!(log.contains("/state/slot-01/evidence/requests/r-request-cli-docker/operations/"));
    assert!(log.contains("GPT_WEBAI_STATE_DIR=/state/slot-01"));
    assert!(log.contains("GPT_WEBAI_ARTIFACTS_DIR=/broker-artifacts/r-request-cli-docker"));
    assert!(!log.contains(&fixture.prompt.display().to_string()));
    assert!(!log.contains(&fixture.upload_one.display().to_string()));
    assert!(!log.contains(&fixture.upload_two.display().to_string()));
    assert!(log.contains("stop gpt-webai-slot-01"));

    let staged_prompt = fixture
        .root
        .join("slots/slot-01/prompts/run-cli-docker/prompt.txt");
    assert_eq!(
        fs::read_to_string(staged_prompt).expect("staged prompt"),
        "hello from fake docker cli run"
    );
    let attachment_names = sorted_file_names(
        &fixture
            .root
            .join("slots/slot-01/attachments/run-cli-docker"),
    );
    assert_eq!(attachment_names.len(), 2);
    assert!(attachment_names[0].starts_with("001-"));
    assert!(attachment_names[0].ends_with(".txt"));
    assert!(attachment_names[1].starts_with("002-"));
    assert!(attachment_names[1].ends_with(".md"));

    let runtime_identity =
        fs::read_to_string(fixture.root.join("runtime-identity")).expect("runtime identity");
    let identity_parts = runtime_identity.lines().collect::<Vec<_>>();
    assert_eq!(identity_parts.len(), 3);
    assert!(identity_parts[0].starts_with("owner_"));
    assert_eq!(identity_parts[1], "1");
    assert!(identity_parts[2].starts_with("runtime_"));

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
            EventType::RuntimeOwnershipGranted,
            EventType::RootCaptureStarted,
            EventType::RootCaptureObserved,
            EventType::ModelSelectionVerified,
            EventType::UploadCompleted,
            EventType::SendClickArmed,
            EventType::SendClicked,
            EventType::TurnStartConfirmed,
            EventType::SessionBindingEstablished,
            EventType::RunningProjected,
            EventType::SessionRebindStarted,
            EventType::SessionRebound,
            EventType::SessionHydrated,
            EventType::PollStarted,
            EventType::AnswerTerminal,
            EventType::ArtifactControlsAbsent,
            EventType::ArtifactClaimCompleted,
            EventType::TerminalPersisted,
            EventType::OutputPublished,
            EventType::RuntimeStopStarted,
            EventType::RuntimeStopped,
            EventType::SessionOperationClaimReleased,
            EventType::RequestClaimReleased,
            EventType::SlotLeaseReleased,
            EventType::RuntimeOwnershipReleased,
            EventType::ReleaseCleanupCommitted,
            EventType::SlotStandbyWritten,
            EventType::ReleaseFinalized,
        ],
    ));
    let journal_text = events
        .iter()
        .map(|event| serde_json::to_string(event).expect("event json"))
        .collect::<String>();
    assert!(!journal_text.contains("WEB:"));
}

#[test]
fn cli_run_release_pipeline_error_still_emits_the_r13_failure_envelope() {
    let fixture = Fixture::new("release-pipeline-error");
    let docker = fixture.write_fake_docker();

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS", "1000")
        .env("GPT_WEBAI_TEST_BREAK_RELEASE_JOURNAL", "1")
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
            "--request-id",
            "request-release-pipeline-error",
            "--run-id",
            "run-release-pipeline-error",
            "--fencing-token",
            "token-release-pipeline-error",
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
        .expect("run CLI with release-pipeline failure");

    fs::set_permissions(
        fixture.root.join("journal/events"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("restore journal permissions");

    assert_eq!(
        output.status.code(),
        Some(70),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "run");
    assert_eq!(value["resultKind"], "run.release_failed");
    assert_eq!(value["status"], "run.release_failed");
    assert_eq!(value["ok"], false);
    assert_eq!(value["terminal"], true);
    assert_eq!(value["reason"], "run.release_failed");
    assert_eq!(value["requestId"], "request-release-pipeline-error");
    assert_eq!(value["runId"], "run-release-pipeline-error");
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(value["sessionId"], "sid-cli-docker");
    assert!(value["eventIds"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
    assert!(value["receiptIds"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
    assert!(value["message"]
        .as_str()
        .is_some_and(|message| message.contains("release pipeline failed")));
}

#[test]
fn cli_run_legacy_wrapper_surface_uses_rust_docker_slot_path() {
    let fixture = Fixture::new("legacy-wrapper");
    let docker = fixture.write_fake_docker();

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS", "1000")
        .args([
            "run",
            "--kind",
            "pro",
            "--prompt",
            "legacy wrapper prompt",
            "--file",
        ])
        .arg(&fixture.upload_one)
        .args(["--docker-bin"])
        .arg(&docker)
        .args([
            "--request-id",
            "request-legacy-wrapper",
            "--run-id",
            "run-legacy-wrapper",
            "--fencing-token",
            "token-legacy-wrapper",
            "--provider-timeout-ms",
            "500000",
            "--runtime-stop-timeout-ms",
            "1000",
            "--runtime-start-timeout-ms",
            "1000",
        ])
        .output()
        .expect("run legacy cli");

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}\nprovider_operations={}\ndocker_log={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.provider_operations).unwrap_or_default(),
        fs::read_to_string(&fixture.docker_log).unwrap_or_default()
    );
    let value = stdout_json(&output.stdout);
    assert_eq!(value["ok"], true);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["resultKind"], "run.terminal_optional_zero");
    assert_eq!(value["status"], "run.terminal_optional_zero");
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(value["sessionId"], "sid-cli-docker");
    assert_eq!(
        value["conversationUrl"],
        "https://chatgpt.com/c/sid-cli-docker"
    );
    assert_eq!(value["answerText"], "final answer");
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);

    let operations = fs::read_to_string(&fixture.provider_operations).expect("provider log");
    assert_eq!(
        operations,
        "status\ncapture.root\nensure-model\nupload-only\nsend-click\nsession-rebind\npoll\nartifact-discover\n"
    );
    let log = fs::read_to_string(&fixture.docker_log).expect("docker log");
    assert!(log.contains("node provider/chatgpt-playwright/cli.mjs --request-file"));
    assert!(log.contains("/state/slot-01/evidence/requests/r-request-legacy-wrapper/operations/"));
    assert!(log.contains("stop gpt-webai-slot-01"));

    let staged_prompt = fixture
        .root
        .join("slots/slot-01/prompts/run-legacy-wrapper/prompt.txt");
    assert_eq!(
        fs::read_to_string(staged_prompt).expect("staged prompt"),
        "legacy wrapper prompt"
    );
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

fn sorted_file_names(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .expect("attachment dir")
        .map(|entry| {
            entry
                .expect("attachment entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}
