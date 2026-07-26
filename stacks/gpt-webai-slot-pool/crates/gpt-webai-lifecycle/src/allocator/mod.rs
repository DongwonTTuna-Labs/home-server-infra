pub mod cursors;
pub mod health;
pub mod scan;

pub const COHORT_ORDER: [&str; 3] = ["cohort-a", "cohort-b", "cohort-c"];
pub const COHORT_A_SLOTS: [&str; 3] = ["slot-01", "slot-02", "slot-03"];
pub const COHORT_B_SLOTS: [&str; 4] = ["slot-04", "slot-05", "slot-06", "slot-07"];
pub const COHORT_C_SLOTS: [&str; 3] = ["slot-08", "slot-09", "slot-10"];
pub const CANONICAL_SLOTS: [&str; 10] = [
    "slot-01", "slot-02", "slot-03", "slot-04", "slot-05", "slot-06", "slot-07", "slot-08",
    "slot-09", "slot-10",
];

pub fn slots_of(cohort: &str) -> Option<&'static [&'static str]> {
    match cohort {
        "cohort-a" => Some(&COHORT_A_SLOTS),
        "cohort-b" => Some(&COHORT_B_SLOTS),
        "cohort-c" => Some(&COHORT_C_SLOTS),
        _ => None,
    }
}

pub fn cohort_of(slot_id: &str) -> Option<&'static str> {
    COHORT_ORDER
        .into_iter()
        .find(|cohort| slots_of(cohort).is_some_and(|slots| slots.contains(&slot_id)))
}

pub fn classify_slot(
    state: &crate::contracts::projection::ProjectionState,
    slot_id: &str,
    now_ms: u64,
) -> Result<Option<scan::SkipReason>, String> {
    use scan::SkipReason;

    if cohort_of(slot_id).is_none() {
        return Ok(Some(SkipReason::StateInvalid));
    }
    if state
        .leases
        .values()
        .any(|record| record.subject_id == slot_id && record.status == "active")
    {
        return Ok(Some(SkipReason::Leased));
    }
    if state
        .runtime_owners
        .values()
        .any(|record| record.cas.subject_id == slot_id && record.cas.status == "active")
    {
        return Ok(Some(SkipReason::RuntimeOwned));
    }
    if let Some(slot) = state.slots.get(slot_id) {
        if slot.cooldown_until_ms.is_some_and(|until| until > now_ms) {
            return Ok(Some(SkipReason::Cooldown));
        }
        if !slot.allocatable && !slot.standby {
            return Ok(Some(SkipReason::HealthBlocked));
        }
    }
    Ok(None)
}
