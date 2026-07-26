use std::fs;
use std::time::Duration;

use crate::request::visual_gate::retry::fixtures::{
    captured, command_count, done, ready_status, sent, status, write_visual_provider,
};
use crate::support::{ready_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::sessions::read_session_record;

#[test]
fn transient_unreachable_visual_gate_retries_before_send() {
    let mut fixture = FakeRun::new("visual-gate-retry");
    let first_status = fixture.write_json("unreachable.json", status("unreachable"));
    let ready_status = fixture.write_json("ready.json", ready_status());
    let capture = fixture.write_json("capture.json", captured());
    let send_json = fixture.write_json("send.json", sent("sid-visual-retry"));
    let poll_json = fixture.write_json("poll.json", done("sid-visual-retry"));
    fixture.provider = write_visual_provider(
        fixture.path(),
        &fixture.args_log,
        &first_status,
        &ready_status,
        &capture,
    );

    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.pre_send_visual_gate = true;
    input.send_retry_delays = vec![Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(output.session_id.as_deref(), Some("sid-visual-retry"));
    assert_eq!(output.send_attempts, 1);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "status"), 2);
    assert_eq!(command_count(&args, "capture"), 2);
    assert_eq!(command_count(&args, "send "), 1);
}

#[test]
fn login_required_visual_gate_blocks_send_without_retry() {
    let mut fixture = FakeRun::new("visual-gate-login");
    let first_status = fixture.write_json("login.json", status("login_required"));
    let ready_status = fixture.write_json("ready.json", ready_status());
    let capture = fixture.write_json("capture.json", captured());
    let send_json = fixture.write_json("send.json", sent("sid-should-not-send"));
    let poll_json = fixture.write_file("unused-poll.json", "{}");
    fixture.provider = write_visual_provider(
        fixture.path(),
        &fixture.args_log,
        &first_status,
        &ready_status,
        &capture,
    );

    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.pre_send_visual_gate = true;
    input.send_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("visual_gate.failed"));
    assert_eq!(output.send_attempts, 0);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "status"), 1);
    assert_eq!(command_count(&args, "capture"), 1);
    assert_eq!(command_count(&args, "send "), 0);
}

#[test]
fn subscription_required_visual_gate_marks_slot_and_reselects_fresh_slot() {
    let mut fixture = FakeRun::new("visual-gate-subscription-reselect");
    let subscription_status =
        fixture.write_json("subscription.json", status("subscription_required"));
    let ready_status = fixture.write_json("ready.json", ready_status());
    let capture = fixture.write_json("capture.json", captured());
    let send_json = fixture.write_json("send.json", sent("sid-visual-reselect"));
    let poll_json = fixture.write_json("poll.json", done("sid-visual-reselect"));
    fixture.provider = write_visual_provider(
        fixture.path(),
        &fixture.args_log,
        &subscription_status,
        &ready_status,
        &capture,
    );

    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.pre_send_visual_gate = true;

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(output.session_id.as_deref(), Some("sid-visual-reselect"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-02"));
    assert_eq!(output.send_attempts, 1);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "status"), 2);
    assert_eq!(command_count(&args, "capture"), 2);
    assert_eq!(command_count(&args, "send "), 1);
    let slot_01_state =
        fs::read_to_string(fixture.path().join("slots/slot-01.state")).expect("slot state");
    assert_eq!(slot_01_state, "status=auth.needs_pro\n");
    let session = read_session_record(fixture.path(), "sid-visual-reselect").expect("session");
    assert_eq!(session.slot_id, "slot-02");
}

#[test]
fn provider_limit_visual_gate_marks_slot_and_reselects_fresh_slot() {
    let mut fixture = FakeRun::new("visual-gate-provider-limit-reselect");
    let provider_limit_status = fixture.write_json("provider-limit.json", status("provider_limit"));
    let ready_status = fixture.write_json("ready.json", ready_status());
    let capture = fixture.write_json("capture.json", captured());
    let send_json = fixture.write_json("send.json", sent("sid-provider-limit-reselect"));
    let poll_json = fixture.write_json("poll.json", done("sid-provider-limit-reselect"));
    fixture.provider = write_visual_provider(
        fixture.path(),
        &fixture.args_log,
        &provider_limit_status,
        &ready_status,
        &capture,
    );

    let mut input = fixture.input(InputSpec {
        send_json,
        poll_json,
        download_json: None,
        files: Vec::new(),
    });
    input.pre_send_visual_gate = true;

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(output.ok);
    assert_eq!(
        output.session_id.as_deref(),
        Some("sid-provider-limit-reselect")
    );
    assert_eq!(output.slot_id.as_deref(), Some("slot-02"));
    assert_eq!(output.send_attempts, 1);
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "status"), 2);
    assert_eq!(command_count(&args, "capture"), 2);
    assert_eq!(command_count(&args, "send "), 1);
    let slot_01_state =
        fs::read_to_string(fixture.path().join("slots/slot-01.state")).expect("slot state");
    assert!(slot_01_state.contains("status=provider.limit\n"));
    assert!(slot_01_state.contains("provider_limit_next_retry_at_ms="));
    let session =
        read_session_record(fixture.path(), "sid-provider-limit-reselect").expect("session");
    assert_eq!(session.slot_id, "slot-02");
}
