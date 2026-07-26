use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::sessions::read_session_record;
use serde_json::json;

use super::fixtures::{exited_runtime, sent};

#[test]
fn fake_provider_poll_failure_marks_session_released_and_releases_slot() {
    let fixture = FakeRun::new("poll-failure");
    let send_json = fixture.write_json("send.json", sent("sid-poll-failure"));
    let poll_json = fixture.write_file("poll.txt", "{not-json");

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
    assert_eq!(output.reason.as_deref(), Some("provider.poll_failed"));
    assert_eq!(output.send_status.as_deref(), Some("sent"));
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-poll-failure").expect("session");
    assert_eq!(session.session_id, "sid-poll-failure");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn fake_terminal_answer_unconfirmed_marks_session_released_and_releases_slot() {
    let fixture = FakeRun::new("answer-unconfirmed");
    let send_json = fixture.write_json("send.json", sent("sid-answer-unconfirmed"));
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "done",
            "sessionId": "sid-answer-unconfirmed",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-answer-unconfirmed",
            "answerText": "",
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
        &ready_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("answer.unconfirmed"));
    assert_eq!(output.send_status.as_deref(), Some("sent"));
    assert_eq!(output.poll_status.as_deref(), Some("done"));
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-answer-unconfirmed").expect("session");
    assert_eq!(session.session_id, "sid-answer-unconfirmed");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn provider_limit_poll_status_marks_slot_limited_and_releases_session() {
    let fixture = FakeRun::new("poll-provider-limit");
    let send_json = fixture.write_json("send.json", sent("sid-poll-provider-limit"));
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "provider_limit",
            "reason": "provider.limit",
            "sessionId": "sid-poll-provider-limit",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-poll-provider-limit",
            "answerText": "visible answer text must not convert provider_limit into done",
            "assistantTurn": {
                "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "diagnostics": [{
                "label": "poll-terminal-before-artifacts",
                "readinessSignals": {
                    "limit": true
                }
            }]
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
    assert_eq!(output.reason.as_deref(), Some("provider.limit"));
    assert_eq!(output.poll_status.as_deref(), Some("provider_limit"));
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session = read_session_record(fixture.path(), "sid-poll-provider-limit").expect("session");
    assert_eq!(session.session_id, "sid-poll-provider-limit");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn scroll_unverified_poll_status_releases_session_without_terminal_success() {
    let fixture = FakeRun::new("poll-scroll-unverified");
    let send_json = fixture.write_json("send.json", sent("sid-poll-scroll-unverified"));
    let poll_json = fixture.write_json(
        "poll.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "session.running_unverified",
            "reason": "scroll.bottom_unverified",
            "sessionId": "sid-poll-scroll-unverified",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-poll-scroll-unverified",
            "diagnostics": [{
                "label": "poll-start-before-wait",
                "scrollBottomProof": {
                    "schema": "gpt-webai.scroll-bottom-proof.v1",
                    "status": "unverified",
                    "reason": "right_edge_scrollbar_thumb_bottom_gap"
                }
            }]
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
    assert_eq!(output.reason.as_deref(), Some("scroll.bottom_unverified"));
    assert_eq!(
        output.poll_status.as_deref(),
        Some("session.running_unverified")
    );
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let session =
        read_session_record(fixture.path(), "sid-poll-scroll-unverified").expect("session");
    assert_eq!(session.session_id, "sid-poll-scroll-unverified");
    assert!(session.updated_at_ms >= session.created_at_ms);
}

#[test]
fn no_allocatable_slot_queues_without_lock_or_provider_invocation() {
    let fixture = FakeRun::new("queued");
    let send_json = fixture.write_json("send.json", json!({}));
    let poll_json = fixture.write_json("poll.json", json!({}));

    let output = run_provider_round_trip(
        fixture.input(InputSpec {
            send_json,
            poll_json,
            download_json: None,
            files: Vec::new(),
        }),
        &exited_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.status, "queued");
    assert_eq!(output.reason.as_deref(), Some("slot.pool_busy"));
    assert!(!output.lock_acquired);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    assert!(!fixture.args_log.exists());
}
