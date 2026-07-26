use std::fs;
use std::time::Duration;

use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use serde_json::json;

use super::fixtures::command_count;

#[test]
fn sent_root_url_is_start_unconfirmed_and_does_not_retry() {
    let fixture = FakeRun::new("send-root-unconfirmed");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "sent",
            "sessionId": "sid-root-url",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/",
            "turnEvidence": {
                "activeTurn": true,
                "userTurnId": format!("turn_{}", "1".repeat(64)),
                "assistantTurnId": format!("turn_{}", "2".repeat(64))
            }
        }),
    );
    let poll_json = fixture.write_file("unused-poll.json", "{}");
    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.send_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("session.start_unconfirmed"));
    assert_eq!(output.session_id.as_deref(), Some("sid-root-url"));
    assert_eq!(
        output.conversation_url.as_deref(),
        Some("https://chatgpt.com/")
    );
    assert_eq!(output.send_attempts, 1);
    assert!(output.send_retry_delays_ms.is_empty());
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 0);
}
