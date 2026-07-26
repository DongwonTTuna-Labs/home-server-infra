use crate::contracts::projection::{AllocatorRecord, WithinCursors};

use super::{slots_of, COHORT_ORDER};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub cohort: String,
    pub slot_id: String,
    pub cohort_cursor_before: u8,
    pub within_cursor_before: u8,
}

impl AllocatorRecord {
    pub fn zeroed(last_event_id: String) -> Self {
        Self {
            cohort_cursor: 0,
            within_cursors: WithinCursors {
                cohort_a: 0,
                cohort_b: 0,
                cohort_c: 0,
            },
            last_scan_ordinal: None,
            last_event_id,
        }
    }
}

pub fn advance(record: &mut AllocatorRecord, scan_ordinal: u8) -> Candidate {
    let cohort_cursor_before = record.cohort_cursor;
    let cohort = COHORT_ORDER[usize::from(record.cohort_cursor)];
    let within_cursor_before = cursor(record, cohort);
    let slots = slots_of(cohort).expect("fixed cohort");
    let slot_id = slots[usize::from(within_cursor_before)];
    set_cursor(
        record,
        cohort,
        (usize::from(within_cursor_before) + 1) % slots.len(),
    );
    record.cohort_cursor = (record.cohort_cursor + 1) % 3;
    record.last_scan_ordinal = Some(scan_ordinal);
    Candidate {
        cohort: cohort.to_string(),
        slot_id: slot_id.to_string(),
        cohort_cursor_before,
        within_cursor_before,
    }
}

fn cursor(record: &AllocatorRecord, cohort: &str) -> u8 {
    match cohort {
        "cohort-a" => record.within_cursors.cohort_a,
        "cohort-b" => record.within_cursors.cohort_b,
        "cohort-c" => record.within_cursors.cohort_c,
        _ => unreachable!("fixed cohort"),
    }
}

fn set_cursor(record: &mut AllocatorRecord, cohort: &str, value: usize) {
    let value = u8::try_from(value).expect("fixed cursor range");
    match cohort {
        "cohort-a" => record.within_cursors.cohort_a = value,
        "cohort-b" => record.within_cursors.cohort_b = value,
        "cohort-c" => record.within_cursors.cohort_c = value,
        _ => unreachable!("fixed cohort"),
    }
}
