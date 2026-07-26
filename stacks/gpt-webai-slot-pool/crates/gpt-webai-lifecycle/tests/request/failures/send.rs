use std::fs;

use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::sessions::read_session_record;
use serde_json::json;

#[test]
fn fake_provider_send_start_unconfirmed_still_releases_slot() {
    let fixture = FakeRun::new("send-failure");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "sent",
            "sessionId": "sid-run",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/other-session",
            "turnEvidence": {
                "activeTurn": true,
                "userTurnId": format!("turn_{}", "1".repeat(64)),
                "assistantTurnId": format!("turn_{}", "2".repeat(64))
            }
        }),
    );
    let poll_json = fixture.write_json("poll.json", json!({}));

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
    assert_eq!(output.reason.as_deref(), Some("session.start_unconfirmed"));
    assert_eq!(output.send_status.as_deref(), Some("sent"));
    assert_eq!(output.send_attempts, 1);
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    assert!(read_session_record(fixture.path(), "sid-run").is_err());
}

#[test]
fn provider_limit_send_status_marks_slot_limited_after_release() {
    let fixture = FakeRun::new("send-provider-limit");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "provider_limit",
            "reason": "provider.limit",
            "diagnostics": [{
                "label": "send-readiness",
                "readinessSignals": { "limit": true }
            }]
        }),
    );
    let poll_json = fixture.write_json("poll.json", json!({}));

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
    assert_eq!(output.reason.as_deref(), Some("send.confirmation_failed"));
    assert_eq!(output.send_status.as_deref(), Some("provider_limit"));
    assert!(output.lock_acquired);
    assert!(output.lock_released);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let state = fs::read_to_string(fixture.path().join("slots/slot-01.state"))
        .expect("slot state after provider-limit send status");
    assert!(state.contains("status=provider.limit\n"));
    assert!(state.contains("provider_limit_observed_at_ms="));
    assert!(state.contains("provider_limit_next_retry_at_ms="));
}
