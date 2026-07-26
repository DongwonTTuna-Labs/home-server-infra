use std::fs;
use std::time::Duration;

use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;

use super::fixtures::{
    command_count, done, sent, write_always_bad_provider, write_sequence_provider,
};

#[test]
fn no_session_send_failure_retries_fresh_slot_and_records_timeline() {
    let mut fixture = FakeRun::new("send-retry-no-session");
    let send_success = fixture.write_json("send-success.json", sent("sid-after-retry"));
    let poll_json = fixture.write_json("poll.json", done("sid-after-retry"));
    let unused_send = fixture.write_file("unused-send.json", "{}");
    fixture.provider = write_sequence_provider(fixture.path(), &fixture.args_log, &send_success);

    let mut input = fixture.input(InputSpec {
        send_json: unused_send,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.send_retry_delays = vec![Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(output.session_id.as_deref(), Some("sid-after-retry"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-02"));
    assert_eq!(output.send_attempts, 2);
    assert_eq!(output.send_retry_delays_ms, vec![0]);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 2);
    assert_eq!(command_count(&args, "poll "), 1);
}

#[test]
fn no_session_send_retry_exhaustion_returns_unknown_session() {
    let mut fixture = FakeRun::new("send-retry-exhausted");
    let poll_json = fixture.write_file("unused-poll.json", "{}");
    let unused_send = fixture.write_file("unused-send.json", "{}");
    fixture.provider = write_always_bad_provider(fixture.path(), &fixture.args_log);

    let mut input = fixture.input(InputSpec {
        send_json: unused_send,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.send_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("send.unknown_session"));
    assert_eq!(output.send_attempts, 3);
    assert_eq!(output.send_retry_delays_ms, vec![0, 0]);
    assert!(output.session_id.is_none());
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 3);
    assert_eq!(command_count(&args, "poll "), 0);
}
