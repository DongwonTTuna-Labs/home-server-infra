use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::locks::{acquire_slot_lease, release_stale_slot_lease};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::provider_runner::ProviderExecution;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe,
};
use gpt_webai_lifecycle::sessions::{read_request_session_record, read_session_record};
use gpt_webai_lifecycle::slots::SlotConfig;
use serde_json::json;

#[test]
fn fake_provider_round_trip_records_terminal_session_and_releases_slot() {
    let fixture = FakeRun::new("success");
    let canary_file = fixture.write_file("canary.txt", "canary");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "sent",
            "sessionId": "sid-run",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-run",
            "turnEvidence": {
                "activeTurn": true,
                "userTurnId": format!("turn_{}", "1".repeat(64)),
                "assistantTurnId": format!("turn_{}", "2".repeat(64))
            }
        }),
    );
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "done",
            "sessionId": "sid-run",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-run",
            "answerText": "final answer",
            "assistantTurn": {
                "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }
        }),
    );

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: None,
            files: vec![canary_file],
        }),
        &ready_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert_eq!(output.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(output.session_id.as_deref(), Some("sid-run"));
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-run").expect("session record");
    assert_eq!(session.session_id, "sid-run");
    assert_eq!(session.cohort, "cohort-a");
    assert!(session.updated_at_ms >= session.created_at_ms);
    assert_eq!(
        read_request_session_record(fixture.path(), "request-run").expect("request session"),
        session
    );
    let answer_dir = fixture.path().join("requests/run-a/artifacts");
    assert_eq!(
        fs::read_to_string(answer_dir.join("answer.md")).expect("answer markdown"),
        "final answer"
    );
    let answer_json = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(answer_dir.join("answer.json")).expect("answer json"),
    )
    .expect("parse answer json");
    assert_eq!(answer_json["sessionId"], "sid-run");
    assert_eq!(
        answer_json["conversationUrl"],
        "https://chatgpt.com/c/sid-run"
    );
    assert_eq!(answer_json["answerText"], "final answer");
    assert_eq!(answer_json["answerTextLen"], "final answer".len());
    assert_eq!(
        answer_json["answerTextSha256"],
        "89cc8a2763c6c9b7cbc8058d68c260aedc026dba2b3a47b4e2cb44fcb8747efe"
    );
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert!(args.contains("send --prompt-file"));
    assert!(args.contains("poll --session sid-run --timeout 30 --artifact-expectation optional"));
}

#[test]
fn slot_lock_race_reselects_another_allocatable_slot_before_send() {
    let fixture = FakeRun::new("slot-lock-race");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "sent",
            "sessionId": "sid-lock-race",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-lock-race",
            "turnEvidence": {
                "activeTurn": true,
                "userTurnId": format!("turn_{}", "1".repeat(64)),
                "assistantTurnId": format!("turn_{}", "2".repeat(64))
            }
        }),
    );
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "done",
            "sessionId": "sid-lock-race",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-lock-race",
            "answerText": "final answer",
            "assistantTurn": {
                "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }
        }),
    );

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: None,
            files: Vec::new(),
        }),
        &LockRaceRuntime {
            state_root: fixture.path().to_path_buf(),
            triggered: AtomicBool::new(false),
        },
    );

    release_stale_slot_lease(fixture.path(), "slot-01").expect("release injected stale lease");
    assert!(output.ok);
    assert_eq!(output.slot_id.as_deref(), Some("slot-02"));
    assert_eq!(output.session_id.as_deref(), Some("sid-lock-race"));
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
}

#[test]
fn pre_poll_wait_gate_recovers_saved_capture_after_provider_timeout() {
    let fixture = FakeRun::new("pre-poll-capture-timeout-recovery");
    let session_id = "sid-pre-poll-recovered";
    let artifacts_dir = fixture.path().join("provider-artifacts");
    write_saved_pre_poll_wait_gate_artifacts(&artifacts_dir, session_id);
    let send_json = fixture.write_json(
        "send.json",
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
        }),
    );
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "done",
            "sessionId": session_id,
            "targetId": "target-run",
            "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
            "answerText": "CANARY_OK_1: recovered after capture timeout",
            "assistantTurn": {
                "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }
        }),
    );

    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.pre_poll_wait_gate = true;
    input.send_process_timeout = Duration::from_millis(100);
    if let ProviderExecution::Host(host) = &mut input.provider_execution {
        host.env.push((
            "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
            artifacts_dir.display().to_string(),
        ));
        host.env.push((
            "GPT_WEBAI_ARTIFACTS_DIR".to_string(),
            artifacts_dir.display().to_string(),
        ));
        host.env.push((
            "FAKE_PROVIDER_CAPTURE_SLEEP_SECONDS".to_string(),
            "0.3".to_string(),
        ));
    }

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok, "output: {output:?}");
    assert_eq!(output.status, "done");
    assert_eq!(output.send_status.as_deref(), Some("sent"));
    assert_eq!(output.poll_status.as_deref(), Some("done"));
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert!(args.contains(&format!(
        "capture --label pre-poll-wait-gate --session {session_id}"
    )));
    assert!(args.contains(&format!(
        "poll --session {session_id} --timeout 30 --artifact-expectation optional"
    )));
}

fn write_saved_pre_poll_wait_gate_artifacts(root: &Path, session_id: &str) {
    let diagnostics_dir = root.join("diagnostics");
    fs::create_dir_all(&diagnostics_dir).expect("create diagnostics dir");
    let label = "pre-poll-wait-gate";
    let screenshot_path = diagnostics_dir.join(format!("{label}.png"));
    let crop_path = diagnostics_dir.join(format!("{label}.right-edge-scrollbar.png"));
    let dom_path = diagnostics_dir.join(format!("{label}.dom.json"));
    let proof_path = diagnostics_dir.join(format!("{label}.scroll-proof.json"));
    fs::write(&screenshot_path, b"png-placeholder").expect("write screenshot");
    fs::write(&crop_path, b"crop-placeholder").expect("write crop");
    let proof = json!({
        "schema": "gpt-webai.scroll-bottom-proof.v1",
        "status": "verified",
        "verificationMode": "strict_visible_right_edge_scrollbar",
        "fullViewportScreenshot": {
            "status": "saved",
            "path": screenshot_path.display().to_string()
        },
        "rightEdgeScrollbarCrop": {
            "status": "saved",
            "path": crop_path.display().to_string()
        },
        "visualScrollbarProof": {
            "status": "right_edge_scrollbar_at_bottom",
            "alignment": {
                "status": "bottom_aligned",
                "thumbBottomGapPx": 4,
                "allowedBottomGapPx": 14
            }
        },
        "visibleRightEdgeScrollbarProof": {
            "status": "verified",
            "method": "strict_visible_right_edge_scrollbar",
            "observations": {
                "screenshot": "right_edge_scrollbar_at_bottom",
                "dom": "right_edge_scrollbar_at_bottom",
                "pixel": "right_edge_scrollbar_at_bottom"
            }
        },
        "moreContentAffordances": {
            "status": "clear",
            "count": 0,
            "samples": []
        },
        "consistency": {
            "status": "consistent",
            "screenshotSelected": {
                "selectionKind": "chatgpt_scroll_root_scrollbar"
            },
            "domSelected": {
                "selectionKind": "chatgpt_scroll_root_scrollbar"
            }
        }
    });
    let dom = json!({
        "schema": "gpt-webai.dom-diagnostics.v1",
        "label": label,
        "sessionId": session_id,
        "url": format!("https://chatgpt.com/c/{session_id}"),
        "title": "ChatGPT",
        "stopControls": [],
        "assistantTurns": [{
            "index": 0,
            "textLength": 42,
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }],
        "userTurns": [{
            "index": 0,
            "textLength": 12,
            "textSha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        }],
        "dialogs": [],
        "providerLimitSurfaces": [],
        "fullViewportScreenshot": proof["fullViewportScreenshot"].clone(),
        "rightEdgeScrollbarCrop": proof["rightEdgeScrollbarCrop"].clone(),
        "scrollBottomProof": proof.clone()
    });
    fs::write(&dom_path, format!("{dom}\n")).expect("write dom");
    fs::write(&proof_path, format!("{proof}\n")).expect("write proof");
}

struct LockRaceRuntime {
    state_root: PathBuf,
    triggered: AtomicBool,
}

impl RuntimeProbe for LockRaceRuntime {
    fn observe(&self, slot: &SlotConfig) -> RuntimeObservation {
        if slot.slot_id.0 == "slot-02" && !self.triggered.swap(true, Ordering::SeqCst) {
            acquire_slot_lease(
                &self.state_root,
                "slot-01",
                "racing-request",
                "racing-run",
                "racing-fence",
                0,
            )
            .expect("inject slot-01 race lease");
        }
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::Ready,
        }
    }
}
