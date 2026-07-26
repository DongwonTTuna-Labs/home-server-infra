use std::collections::BTreeMap;

use serde::Serialize;

use crate::config::SupervisorConfig;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlotId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccountGroupId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlotConfig {
    pub slot_id: SlotId,
    pub index: u8,
    pub container: String,
    pub cdp_port: u16,
    pub account_group: AccountGroupId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationCandidate {
    pub slot_id: SlotId,
    pub account_group: AccountGroupId,
    pub allocatable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationDecision {
    pub slot_id: SlotId,
    pub preferred_group: AccountGroupId,
    pub allocated_group: AccountGroupId,
    pub fallback_reason: Option<String>,
}

impl SlotConfig {
    pub fn key_prefix(&self) -> String {
        self.slot_id.0.replace('-', "_")
    }
}

pub fn inventory(config: &SupervisorConfig) -> Vec<SlotConfig> {
    (1..=config.slot_count)
        .map(|index| {
            let slot_id = format!("slot-{index:02}");
            let container = format!("{}{}", config.slot_container_prefix, slot_id);
            SlotConfig {
                slot_id: SlotId(slot_id),
                index,
                container,
                cdp_port: 9222 + u16::from(index),
                account_group: group_for_slot_index(config.slot_count, index),
            }
        })
        .collect()
}

pub fn canonical_inventory(config: &SupervisorConfig) -> Vec<SlotConfig> {
    crate::allocator::CANONICAL_SLOTS
        .into_iter()
        .enumerate()
        .map(|(offset, slot_id)| {
            let index = u8::try_from(offset + 1).expect("canonical slot index");
            SlotConfig {
                slot_id: SlotId(slot_id.to_string()),
                index,
                container: format!("{}{}", config.slot_container_prefix, slot_id),
                cdp_port: 9222 + u16::from(index),
                account_group: AccountGroupId(
                    crate::allocator::cohort_of(slot_id)
                        .expect("canonical slot cohort")
                        .to_string(),
                ),
            }
        })
        .collect()
}

pub fn group_for_slot_index(slot_count: u8, index: u8) -> AccountGroupId {
    let split = slot_count.div_ceil(2);
    if index <= split {
        AccountGroupId("group-01".to_string())
    } else {
        AccountGroupId("group-02".to_string())
    }
}

pub fn next_preferred_group(last_preferred_group: Option<&str>) -> AccountGroupId {
    match last_preferred_group {
        Some("group-01") => AccountGroupId("group-02".to_string()),
        Some("group-02") => AccountGroupId("group-01".to_string()),
        _ => AccountGroupId("group-01".to_string()),
    }
}

pub fn select_fresh_slot(
    candidates: &[AllocationCandidate],
    last_preferred_group: Option<&str>,
) -> Option<AllocationDecision> {
    select_fresh_slot_with_rotation(candidates, last_preferred_group, &BTreeMap::new())
}

pub fn select_fresh_slot_with_rotation(
    candidates: &[AllocationCandidate],
    last_preferred_group: Option<&str>,
    last_allocated_slot_by_group: &BTreeMap<String, String>,
) -> Option<AllocationDecision> {
    let preferred_group = next_preferred_group(last_preferred_group);
    if let Some(candidate) = first_allocatable_in_group_after(
        candidates,
        &preferred_group,
        last_allocated_slot_by_group
            .get(&preferred_group.0)
            .map(String::as_str),
    ) {
        return Some(AllocationDecision {
            slot_id: candidate.slot_id.clone(),
            preferred_group: preferred_group.clone(),
            allocated_group: preferred_group,
            fallback_reason: None,
        });
    }

    let fallback_group = other_group(&preferred_group);
    first_allocatable_in_group_after(
        candidates,
        &fallback_group,
        last_allocated_slot_by_group
            .get(&fallback_group.0)
            .map(String::as_str),
    )
    .map(|candidate| AllocationDecision {
        slot_id: candidate.slot_id.clone(),
        preferred_group,
        allocated_group: fallback_group,
        fallback_reason: Some("preferred_group_unavailable".to_string()),
    })
}

fn first_allocatable_in_group_after<'a>(
    candidates: &'a [AllocationCandidate],
    group: &AccountGroupId,
    last_allocated_slot: Option<&str>,
) -> Option<&'a AllocationCandidate> {
    let group_candidates = candidates
        .iter()
        .filter(|candidate| candidate.account_group == *group)
        .collect::<Vec<_>>();
    if group_candidates.is_empty() {
        return None;
    }
    let start = last_allocated_slot
        .and_then(|slot_id| {
            group_candidates
                .iter()
                .position(|candidate| candidate.slot_id.0 == slot_id)
        })
        .map(|index| index + 1)
        .unwrap_or(0);
    group_candidates
        .iter()
        .cycle()
        .skip(start)
        .take(group_candidates.len())
        .copied()
        .find(|candidate| candidate.allocatable)
}

fn other_group(group: &AccountGroupId) -> AccountGroupId {
    if group.0 == "group-01" {
        AccountGroupId("group-02".to_string())
    } else {
        AccountGroupId("group-01".to_string())
    }
}
