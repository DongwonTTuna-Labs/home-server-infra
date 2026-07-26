use gpt_webai_lifecycle::preflight::run_preflight;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use serde_json::json;

use super::fixtures::{
    ready_runtime, runtime_with_exited_slot, runtime_with_standby_first_ready_second, Fixture,
};

#[test]
fn fake_preflight_emits_the_closed_r13_ready_envelope() {
    let fixture = Fixture::new("ready");
    let outcome = run_preflight(
        fixture.input(Some("slot-01".to_string()), &[]),
        &ready_runtime(),
        "preflight-ready",
    )
    .expect("preflight");

    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.envelope.schema, "gpt-webai.lifecycle.r13.v1");
    assert_eq!(outcome.envelope.command, "preflight");
    assert_eq!(outcome.envelope.result_kind, "preflight.ready");
    assert_eq!(outcome.envelope.run_id.as_deref(), Some("preflight-run"));
    assert_eq!(outcome.envelope.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(outcome.envelope.cohort.as_deref(), Some("cohort-a"));
    assert_eq!(outcome.envelope.receipt_ids.len(), 1);
    assert!(outcome.envelope.event_ids.is_empty());
    assert_clean(&fixture);
}

#[test]
fn fake_preflight_maps_contract_invalid_output_to_schema_drift() {
    let fixture = Fixture::new("schema-drift");
    let outcome = run_preflight(
        fixture.malformed_input(Some("slot-01".to_string())),
        &ready_runtime(),
        "preflight-schema-drift",
    )
    .expect("preflight");

    assert_eq!(outcome.exit_code, 70);
    assert_eq!(outcome.envelope.result_kind, "preflight.schema_drift");
    assert_eq!(
        outcome.envelope.reason.as_deref(),
        Some("contract.invalid_provider_envelope")
    );
    assert_clean(&fixture);
}

#[test]
fn fake_preflight_retries_unknown_once_and_uses_the_second_observation() {
    let fixture = Fixture::new("unknown-retry");
    let frames = [
        json!({"healthStatus":"unknown","composerReady":false,"modelLabel":"unknown"}),
        json!({"healthStatus":"ready","composerReady":true,"modelLabel":"pro"}),
    ];
    let outcome = run_preflight(
        fixture.input(Some("slot-01".to_string()), &frames),
        &ready_runtime(),
        "preflight-unknown-retry",
    )
    .expect("preflight");

    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.envelope.result_kind, "preflight.ready");
    assert_eq!(outcome.envelope.receipt_ids.len(), 2);
    assert_clean(&fixture);
}

#[test]
fn fake_preflight_skips_nonrunning_slots_before_the_r13_probe() {
    let fixture = Fixture::new("skip-standby");
    let outcome = run_preflight(
        fixture.input(None, &[]),
        &runtime_with_standby_first_ready_second(),
        "preflight-skip-standby",
    )
    .expect("preflight");

    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.envelope.result_kind, "preflight.ready");
    assert_eq!(outcome.envelope.slot_id.as_deref(), Some("slot-02"));
    assert_clean(&fixture);
}

#[test]
fn fake_preflight_reports_no_slot_for_a_pinned_nonrunning_slot() {
    let fixture = Fixture::new("pinned-exited");
    let outcome = run_preflight(
        fixture.input(Some("slot-01".to_string()), &[]),
        &runtime_with_exited_slot(),
        "preflight-pinned-exited",
    )
    .expect("preflight");

    assert_eq!(outcome.exit_code, 70);
    assert_eq!(outcome.envelope.result_kind, "preflight.no_slot");
    assert_eq!(
        outcome.envelope.reason.as_deref(),
        Some("preflight.no_slot")
    );
    assert_eq!(outcome.envelope.slot_id.as_deref(), Some("slot-01"));
    assert_clean(&fixture);
}

fn assert_clean(fixture: &Fixture) {
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
}
