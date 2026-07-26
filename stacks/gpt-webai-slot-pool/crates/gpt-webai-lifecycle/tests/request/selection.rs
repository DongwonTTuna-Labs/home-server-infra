use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{
    holder_count, lock_count, read_slot_rotation_cursor, write_group_cursor,
    write_slot_rotation_cursor,
};
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use gpt_webai_lifecycle::slots::AccountGroupId;
use serde_json::json;

use crate::support::{FakeRun, InputSpec};

#[test]
fn request_selection_rotates_within_group_using_persisted_slot_cursor() {
    let fixture = FakeRun::new("request-slot-rotation");
    write_group_cursor(fixture.path(), &AccountGroupId("group-02".to_string()))
        .expect("seed group cursor");
    write_slot_rotation_cursor(
        fixture.path(),
        &AccountGroupId("group-01".to_string()),
        "slot-01",
    )
    .expect("seed slot cursor");
    let send_json = fixture.write_json(
        "send.json",
        json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "sent",
            "sessionId": "sid-slot-rotation",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-slot-rotation",
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
            "sessionId": "sid-slot-rotation",
            "targetId": "target-run",
            "conversationUrl": "https://chatgpt.com/c/sid-slot-rotation",
            "answerText": "final answer",
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
    input.config.slot_count = 4;

    let output = run_provider_round_trip(input, &four_ready_slots_runtime());

    assert!(output.ok);
    assert_eq!(output.slot_id.as_deref(), Some("slot-02"));
    let cursor = read_slot_rotation_cursor(fixture.path(), "group-01")
        .expect("read slot cursor")
        .expect("slot cursor");
    assert_eq!(cursor.last_allocated_slot, "slot-02");
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
}

fn four_ready_slots_runtime() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new((1..=4).map(|index| {
        (
            format!("slot-{index:02}"),
            RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        )
    }))
}
