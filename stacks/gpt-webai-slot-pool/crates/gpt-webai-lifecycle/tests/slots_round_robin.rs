use gpt_webai_lifecycle::config::SupervisorConfig;
use std::collections::BTreeMap;

use gpt_webai_lifecycle::slots::{
    group_for_slot_index, inventory, next_preferred_group, select_fresh_slot,
    select_fresh_slot_with_rotation, AccountGroupId, AllocationCandidate, SlotId,
};

fn candidate(slot: &str, group: &str, allocatable: bool) -> AllocationCandidate {
    AllocationCandidate {
        slot_id: SlotId(slot.to_string()),
        account_group: AccountGroupId(group.to_string()),
        allocatable,
    }
}

#[test]
fn ten_slots_are_split_into_two_five_slot_account_groups() {
    let config = SupervisorConfig {
        state_root: std::path::PathBuf::from("/tmp/nonexistent-gpt-webai-test"),
        slot_count: 10,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 15_000,
    };

    let slots = inventory(&config);
    assert_eq!(slots[0].slot_id.0, "slot-01");
    assert_eq!(slots[0].account_group.0, "group-01");
    assert_eq!(slots[4].slot_id.0, "slot-05");
    assert_eq!(slots[4].account_group.0, "group-01");
    assert_eq!(slots[5].slot_id.0, "slot-06");
    assert_eq!(slots[5].account_group.0, "group-02");
    assert_eq!(slots[9].slot_id.0, "slot-10");
    assert_eq!(slots[9].account_group.0, "group-02");
}

#[test]
fn preferred_group_alternates_between_the_two_account_groups() {
    assert_eq!(next_preferred_group(None).0, "group-01");
    assert_eq!(next_preferred_group(Some("group-01")).0, "group-02");
    assert_eq!(next_preferred_group(Some("group-02")).0, "group-01");
}

#[test]
fn fresh_request_selects_first_allocatable_slot_in_preferred_group() {
    let candidates = vec![
        candidate("slot-01", "group-01", true),
        candidate("slot-06", "group-02", true),
    ];

    let decision = select_fresh_slot(&candidates, Some("group-01")).expect("decision");
    assert_eq!(decision.preferred_group.0, "group-02");
    assert_eq!(decision.allocated_group.0, "group-02");
    assert_eq!(decision.slot_id.0, "slot-06");
    assert_eq!(decision.fallback_reason, None);
}

#[test]
fn fresh_request_falls_back_to_other_group_when_preferred_group_is_full() {
    let candidates = vec![
        candidate("slot-01", "group-01", true),
        candidate("slot-06", "group-02", false),
        candidate("slot-07", "group-02", false),
    ];

    let decision = select_fresh_slot(&candidates, Some("group-01")).expect("fallback decision");
    assert_eq!(decision.preferred_group.0, "group-02");
    assert_eq!(decision.allocated_group.0, "group-01");
    assert_eq!(decision.slot_id.0, "slot-01");
    assert_eq!(
        decision.fallback_reason.as_deref(),
        Some("preferred_group_unavailable")
    );
}

#[test]
fn fresh_request_returns_none_when_no_slot_is_allocatable() {
    let candidates = vec![
        candidate("slot-01", "group-01", false),
        candidate("slot-06", "group-02", false),
    ];

    assert_eq!(select_fresh_slot(&candidates, Some("group-02")), None);
}

#[test]
fn fresh_request_rotates_within_the_preferred_group_after_last_allocated_slot() {
    let candidates = vec![
        candidate("slot-06", "group-02", true),
        candidate("slot-07", "group-02", true),
        candidate("slot-08", "group-02", true),
    ];
    let last_allocated = BTreeMap::from([("group-02".to_string(), "slot-06".to_string())]);

    let decision = select_fresh_slot_with_rotation(&candidates, Some("group-01"), &last_allocated)
        .expect("decision");

    assert_eq!(decision.preferred_group.0, "group-02");
    assert_eq!(decision.allocated_group.0, "group-02");
    assert_eq!(decision.slot_id.0, "slot-07");
}

#[test]
fn fresh_request_wraps_group_rotation_when_last_allocated_slot_is_at_end() {
    let candidates = vec![
        candidate("slot-06", "group-02", true),
        candidate("slot-07", "group-02", true),
        candidate("slot-08", "group-02", true),
    ];
    let last_allocated = BTreeMap::from([("group-02".to_string(), "slot-08".to_string())]);

    let decision = select_fresh_slot_with_rotation(&candidates, Some("group-01"), &last_allocated)
        .expect("decision");

    assert_eq!(decision.slot_id.0, "slot-06");
}

#[test]
fn group_split_helper_keeps_user_visible_five_and_five_boundary() {
    assert_eq!(group_for_slot_index(10, 1).0, "group-01");
    assert_eq!(group_for_slot_index(10, 5).0, "group-01");
    assert_eq!(group_for_slot_index(10, 6).0, "group-02");
    assert_eq!(group_for_slot_index(10, 10).0, "group-02");
}
