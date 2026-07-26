use std::collections::BTreeSet;

use crate::errors::LifecycleError;
use crate::records;
use crate::runtime::RuntimeProbe;
use crate::slots::{select_fresh_slot_with_rotation, AllocationCandidate, AllocationDecision};
use crate::status;

use super::input::RequestRunInput;

pub(crate) fn select_slot_avoiding_groups(
    input: &RequestRunInput,
    runtime: &dyn RuntimeProbe,
    avoided_groups: &BTreeSet<String>,
) -> Result<Option<AllocationDecision>, LifecycleError> {
    let status = status::build_status(&input.config, runtime)?;
    let candidates = status
        .slots
        .iter()
        .map(|slot| AllocationCandidate {
            slot_id: crate::slots::SlotId(slot.slot_id.clone()),
            account_group: crate::slots::AccountGroupId(slot.account_group.clone()),
            allocatable: slot.allocatable && !avoided_groups.contains(&slot.account_group),
        })
        .collect::<Vec<_>>();
    let cursor = records::read_group_cursor(&input.config.state_root)?;
    let slot_rotation = records::read_slot_rotation_cursors(
        &input.config.state_root,
        candidates
            .iter()
            .map(|candidate| candidate.account_group.0.clone()),
    )?;
    Ok(select_fresh_slot_with_rotation(
        &candidates,
        cursor
            .as_ref()
            .map(|record| record.last_preferred_group.as_str()),
        &slot_rotation,
    ))
}

pub(crate) fn persist_allocation_cursors(
    input: &RequestRunInput,
    decision: &AllocationDecision,
) -> std::io::Result<()> {
    records::write_allocation_cursors(
        &input.config.state_root,
        records::AllocationCursorUpdate {
            preferred_group: &decision.preferred_group,
            allocated_group: &decision.allocated_group,
            slot_id: &decision.slot_id.0,
        },
    )
}
