use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::contracts::events::EventType;
use gpt_webai_lifecycle::journal::EventStore;
use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};

#[path = "cli_run/fixtures.rs"]
mod run_fixture;

#[test]
fn cli_show_runs_the_r13_provider_journal_and_release_path() {
    let root = temp_state_root("cli-show-r13");
    let record = new_session_record(NewSessionRecord {
        request_id: Some("request-cli-show".to_string()),
        run_id: Some("run-cli-show".to_string()),
        session_id: "sid-cli-show".to_string(),
        conversation_url: "https://chatgpt.com/c/sid-cli-show".to_string(),
        slot_id: "slot-06".to_string(),
        cohort: "cohort-b".to_string(),
        page_binding_generation: 1,
    })
    .expect("new session");
    write_session_record(&root, &record).expect("write session");
    let provider = write_r13_provider(&root, "running");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("HOME", root.join("home"))
        .args([
            "show",
            "--json",
            "--session",
            "sid-cli-show",
            "--fencing-token",
            "fixture-fence",
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
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "show");
    assert_eq!(value["resultKind"], "show.running");
    assert_eq!(value["sessionId"], "sid-cli-show");
    assert_eq!(value["slotId"], "slot-06");
    assert_eq!(value["cohort"], "cohort-b");
    assert_eq!(
        value["conversationUrl"],
        "https://chatgpt.com/c/sid-cli-show"
    );
    assert_eq!(value["answerPath"], serde_json::Value::Null);
    assert_eq!(value["answerText"], serde_json::Value::Null);
    assert!(value["receiptIds"]
        .as_array()
        .is_some_and(|ids| ids.len() == 2));

    let mut events = EventStore::new(&root).load_all().expect("load journal");
    events.sort_by_key(|event| (event.created_at_ms, event.event_id.clone()));
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(ordered_subsequence(
        &event_types,
        &[
            EventType::SessionOperationClaimGranted,
            EventType::PersistedSessionLeaseGranted,
            EventType::SessionRuntimeOwnershipGranted,
            EventType::SlotHealthProbeStarted,
            EventType::SlotHealthObserved,
            EventType::SessionRebindStarted,
            EventType::SessionRebound,
            EventType::SessionHydrationObserved,
            EventType::SessionHydrated,
            EventType::RuntimeStopStarted,
            EventType::RuntimeStopped,
            EventType::SessionOperationClaimReleased,
            EventType::SlotLeaseReleased,
            EventType::RuntimeOwnershipReleased,
            EventType::ReleaseCleanupCommitted,
            EventType::ReleaseFinalized,
        ],
    ));
    let updated = gpt_webai_lifecycle::sessions::read_session_record(&root, "sid-cli-show")
        .expect("updated session");
    assert_eq!(updated.page_binding_generation, 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_show_publishes_a_new_terminal_answer_without_request_stream_events() {
    let root = temp_state_root("cli-show-terminal-r13");
    let record = new_session_record(NewSessionRecord {
        request_id: Some("request-cli-show-terminal".to_string()),
        run_id: Some("run-cli-show-terminal".to_string()),
        session_id: "sid-cli-show-terminal".to_string(),
        conversation_url: "https://chatgpt.com/c/sid-cli-show-terminal".to_string(),
        slot_id: "slot-06".to_string(),
        cohort: "cohort-b".to_string(),
        page_binding_generation: 1,
    })
    .expect("new session");
    write_session_record(&root, &record).expect("write session");
    let provider = write_r13_provider(&root, "terminal");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("HOME", root.join("home"))
        .args([
            "show",
            "--json",
            "--session",
            "sid-cli-show-terminal",
            "--fencing-token",
            "fixture-fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run terminal show");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["resultKind"], "show.terminal");
    assert_eq!(value["answerText"], "terminal show answer");
    assert_eq!(value["answerSizeBytes"], "terminal show answer".len());
    let answer_path = value["answerPath"].as_str().expect("answer path");
    assert_eq!(
        fs::read_to_string(root.join(answer_path)).expect("persisted show answer"),
        "terminal show answer"
    );
    assert!(value["answerSha256"]
        .as_str()
        .is_some_and(|hash| hash.starts_with("sha256:")));

    let events = EventStore::new(&root).load_all().expect("load journal");
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::SessionHydrated));
    assert!(!events.iter().any(|event| matches!(
        event.event_type,
        EventType::AnswerTerminal | EventType::TerminalPersisted | EventType::OutputPublished
    )));
    let updated =
        gpt_webai_lifecycle::sessions::read_session_record(&root, "sid-cli-show-terminal")
            .expect("updated session");
    assert_eq!(updated.page_binding_generation, 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_show_reports_first_mutation_lock_contention_as_an_r13_envelope() {
    let root = temp_state_root("cli-show-lock-contended");
    let record = new_session_record(NewSessionRecord {
        request_id: Some("request-cli-show-lock".to_string()),
        run_id: Some("run-cli-show-lock".to_string()),
        session_id: "sid-cli-show-lock".to_string(),
        conversation_url: "https://chatgpt.com/c/sid-cli-show-lock".to_string(),
        slot_id: "slot-06".to_string(),
        cohort: "cohort-b".to_string(),
        page_binding_generation: 1,
    })
    .expect("new session");
    write_session_record(&root, &record).expect("write session");
    for relative in ["journal", "journal/locks", "journal/locks/mutation.lock"] {
        let path = root.join(relative);
        fs::create_dir(&path).expect("private contended lock component");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("private contended lock mode");
    }
    let provider = write_r13_provider(&root, "running");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("HOME", root.join("home"))
        .args([
            "show",
            "--json",
            "--session",
            "sid-cli-show-lock",
            "--fencing-token",
            "fixture-fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .output()
        .expect("run lock-contended show");

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "show");
    assert_eq!(value["resultKind"], "show.lock_contended");
    assert_eq!(value["reason"], "lock.contended");
    assert_eq!(value["sessionId"], "sid-cli-show-lock");
    assert_eq!(value["eventIds"], serde_json::json!([]));
    assert!(EventStore::new(&root)
        .load_all()
        .expect("load journal")
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_show_reports_idle_when_terminal_sha_is_already_projected() {
    let fixture = run_fixture::Fixture::new("show-idle-r13");
    let docker = fixture.write_fake_docker();
    let started = Command::new(run_fixture::binary())
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
            "request-cli-show-idle",
            "--run-id",
            "run-cli-show-idle",
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
    let started_value = run_fixture::stdout_json(&started.stdout);
    let session_id = started_value["sessionId"]
        .as_str()
        .expect("session id")
        .to_string();

    let shown = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS", "1000")
        .args(["show", "--json", "--session"])
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
        .expect("run idle show");
    assert!(
        shown.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&shown.stdout),
        String::from_utf8_lossy(&shown.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("show json");
    assert_eq!(value["resultKind"], "show.idle");
    assert_eq!(value["terminal"], false);
    assert_eq!(value["answerPath"], serde_json::Value::Null);
    assert_eq!(value["answerSha256"], serde_json::Value::Null);
    assert_eq!(value["answerText"], serde_json::Value::Null);
    assert_eq!(
        fs::read_to_string(&fixture.provider_operations).expect("provider operations"),
        "status\ncapture.root\nensure-model\nupload-only\nsend-click\nsession-rebind\npoll\nartifact-discover\nstatus\nsession-rebind\n"
    );
    let events = EventStore::new(&fixture.root)
        .load_all()
        .expect("load journal");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::AnswerTerminal)
            .count(),
        1,
        "show must not emit a second AnswerTerminal"
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

fn write_r13_provider(root: &Path, mode: &str) -> PathBuf {
    let provider = root.join("r13-provider.py");
    let source = r#"#!/usr/bin/python3
import hashlib
import json
import os
import sys

mode = "__SHOW_MODE__"

def canonical(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode("utf-8") + b"\n"

def sha(value):
    return hashlib.sha256(value).hexdigest()

def h256(value):
    return "sha256:" + sha(value)

def derived(prefix, value):
    return prefix + "_" + sha(canonical(value))

request_path = sys.argv[sys.argv.index("--request-file") + 1]
with open(request_path, "rb") as stream:
    request = json.load(stream)

operation = request["operation"]
identity = request["identity"]
if operation == "status":
    data = {
        "composerReady": True,
        "dockerStatus": "running",
        "healthStatus": "ready",
        "modelLabel": "pro",
        "retryAfterMs": None,
    }
    status = "done"
elif operation == "session-rebind":
    expectation = request["operationData"]["expectation"]
    generation = expectation["lastKnownPageBindingGeneration"] + 1
    browser_guid = "browser-guid-fixture"
    raw_target = "target-fixture"
    page_id = derived("page", ["pr72.page.r13.v1", browser_guid, raw_target, "main-frame", "loader-fixture"])
    root_hash = h256(canonical({"fixture": "root-binding"}))
    page = {
        "bindingGeneration": generation,
        "bindingId": derived("binding", ["pr72.page-binding.r13.v1", page_id, root_hash]),
        "browserContextId": derived("ctx", ["pr72.ctx.r13.v1", browser_guid, "context-fixture"]),
        "cohort": expectation["cohort"],
        "domMutationGeneration": 1,
        "leaseGeneration": expectation["leaseGeneration"],
        "leaseId": expectation["leaseId"],
        "pageIncarnationId": page_id,
        "rootBindingHash": root_hash,
        "runtimeIncarnationId": expectation["runtimeIncarnationId"],
        "runtimeOwnerGeneration": expectation["runtimeOwnerGeneration"],
        "runtimeOwnerId": expectation["runtimeOwnerId"],
        "slotId": expectation["slotId"],
        "targetId": derived("target", ["pr72.target.r13.v1", browser_guid, raw_target]),
    }
    assistant_turn = derived("turn", ["pr72.turn.r13.v1", expectation["sessionId"], "assistant", "assistant-message-fixture"])
    user_turn = derived("turn", ["pr72.turn.r13.v1", expectation["sessionId"], "user", "user-message-fixture"])
    terminal = mode == "terminal"
    answer = b"terminal show answer"
    answer_sha = h256(answer) if terminal else None
    echo = dict(page)
    echo.update({
        "activeTurn": not terminal,
        "conversationUrl": expectation["conversationUrl"],
        "pageBindingGeneration": generation,
        "requestId": expectation["requestId"],
        "runId": expectation["runId"],
        "sessionBindingId": derived("binding", ["pr72.session-binding.r13.v1", expectation["sessionId"], expectation["slotId"], expectation["cohort"]]),
        "sessionId": expectation["sessionId"],
        "terminalAnswerSha256": answer_sha,
        "visibleAssistantTurnId": assistant_turn,
        "visibleUserTurnId": user_turn,
    })
    evidence = {"mediaType": "application/json", "path": "cdp.sanitized.json", "sha256": h256(b"fake-cdp"), "sizeBytes": 8}
    terminal_answer = None
    if terminal:
        state_root = request_path.split(os.sep + "evidence" + os.sep + "requests" + os.sep, 1)[0]
        request_key = "r-" + identity["requestId"]
        answer_rel = "answers/" + identity["operationId"] + ".answer.md"
        answer_path = os.path.join(state_root, "artifacts", request_key, answer_rel)
        os.makedirs(os.path.dirname(answer_path), mode=0o700, exist_ok=True)
        os.chmod(os.path.dirname(answer_path), 0o700)
        with open(answer_path, "xb") as stream:
            stream.write(answer)
        os.chmod(answer_path, 0o600)
        terminal_answer = {
            "answerRelPath": answer_rel,
            "answerSha256": answer_sha,
            "answerSizeBytes": len(answer),
            "terminalAssistantTurnId": assistant_turn,
        }
    data = {
        "expectation": expectation,
        "failureReason": None,
        "hydrationObservations": [{
            "evidenceRefs": [evidence],
            "observedAtMs": 2,
            "observedEcho": echo,
            "remainingDeadlineMs": 89000,
            "sequenceIndex": 0,
            "state": "answer_visible" if terminal else "active_generation_visible",
        }],
        "observedEcho": echo,
        "pageBindingGeneration": generation,
        "terminalAnswer": terminal_answer,
    }
    status = "done" if terminal else "running"
else:
    raise SystemExit("unexpected operation: " + operation)

receipt = {
    "createdAtMs": 1,
    "operation": operation,
    "operationId": identity["operationId"],
    "payload": data,
    "receiptId": "",
    "requestId": identity["requestId"],
    "runId": identity["runId"],
    "schema": "pr72.receipt.r13.v1",
    "sessionId": identity["sessionId"],
}
receipt["receiptId"] = "receipt_" + sha(canonical(receipt))
receipt_bytes = canonical(receipt)
receipt_path = os.path.join(os.path.dirname(request_path), "provider-receipt.json")
with open(receipt_path, "xb") as stream:
    stream.write(receipt_bytes)
os.chmod(receipt_path, 0o600)
response = {
    "identity": identity,
    "ok": True,
    "operation": operation,
    "operationData": data,
    "providerReason": None,
    "receipt": {
        "mediaType": "application/json",
        "path": "provider-receipt.json",
        "sha256": h256(receipt_bytes),
        "sizeBytes": len(receipt_bytes),
    },
    "schema": "gpt-webai.provider.response.r13.v1",
    "status": status,
}
sys.stdout.buffer.write(canonical(response))
"#
    .replace("__SHOW_MODE__", mode);
    fs::write(&provider, source).expect("provider");
    let mut permissions = fs::metadata(&provider).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&provider, permissions).expect("chmod");
    provider
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
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
