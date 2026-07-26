use gpt_webai_lifecycle::contracts::ids::h256;
use gpt_webai_lifecycle::qa::counters::{
    complete, empty, record_matrix, record_repeat, reset_all, reset_case, QaCounterError,
    REPEAT_CASES,
};
use gpt_webai_lifecycle::qa::fingerprint::{fingerprint_entries, SourceContentEntry};

#[test]
fn matrix_requires_three_consecutive_passes_on_one_fingerprint() {
    let fingerprint = h256(b"source-a");
    let mut record = empty(event('0')).unwrap();
    for iteration in 1..=3 {
        record_matrix(
            &mut record,
            iteration,
            &fingerprint,
            21,
            21,
            &event(char::from_digit(iteration.into(), 10).unwrap()),
        )
        .unwrap();
    }
    assert_eq!(record.matrix_iterations_passed, 3);
    assert!(record_matrix(&mut record, 3, &fingerprint, 21, 21, &event('4')).is_err());
}

#[test]
fn failed_or_out_of_order_repeat_is_rejected_until_case_reset() {
    let fingerprint = h256(b"source-a");
    let mut record = empty(event('0')).unwrap();
    assert!(record_repeat(&mut record, "R04", 1, &fingerprint, false, &event('1')).is_err());
    record_repeat(&mut record, "R04", 1, &fingerprint, true, &event('2')).unwrap();
    assert!(record_repeat(&mut record, "R04", 3, &fingerprint, true, &event('3')).is_err());
    reset_case(&mut record, "R04", &fingerprint, &event('4')).unwrap();
    assert_eq!(record.repeat_counts.get("R04"), Some(&0));
    record_repeat(&mut record, "R04", 1, &fingerprint, true, &event('5')).unwrap();
}

#[test]
fn source_change_requires_explicit_all_reset() {
    let first = h256(b"source-a");
    let second = h256(b"source-b");
    let mut record = empty(event('0')).unwrap();
    record_matrix(&mut record, 1, &first, 21, 21, &event('1')).unwrap();
    assert_eq!(
        record_matrix(&mut record, 2, &second, 21, 21, &event('2')),
        Err(QaCounterError::FingerprintResetRequired)
    );
    reset_all(&mut record, &second, &event('3')).unwrap();
    assert_eq!(record.matrix_iterations_passed, 0);
    assert!(record.repeat_counts.is_empty());
    record_matrix(&mut record, 1, &second, 21, 21, &event('4')).unwrap();
}

#[test]
fn completeness_requires_matrix_three_and_each_named_repeat_ten() {
    let fingerprint = h256(b"source-a");
    let mut record = empty(event('0')).unwrap();
    for iteration in 1..=3 {
        record_matrix(
            &mut record,
            iteration,
            &fingerprint,
            21,
            21,
            &event(char::from_digit(iteration.into(), 10).unwrap()),
        )
        .unwrap();
    }
    for (case_offset, case) in REPEAT_CASES.iter().enumerate() {
        for repetition in 1..=10 {
            record_repeat(
                &mut record,
                case,
                repetition,
                &fingerprint,
                true,
                &event(hex_char((case_offset * 10 + repetition as usize) % 16)),
            )
            .unwrap();
        }
    }
    assert!(complete(&record));
}

#[test]
fn content_fingerprint_is_sorted_and_excludes_evidence_and_build_output() {
    let entries = vec![
        SourceContentEntry {
            path: "Cargo.toml".to_string(),
            sha256: h256(b"cargo"),
            size_bytes: 1,
        },
        SourceContentEntry {
            path: "src/lib.rs".to_string(),
            sha256: h256(b"lib"),
            size_bytes: 2,
        },
    ];
    assert!(fingerprint_entries(&entries).is_ok());
    let mut reversed = entries.clone();
    reversed.reverse();
    assert!(fingerprint_entries(&reversed).is_err());
    assert!(fingerprint_entries(&[SourceContentEntry {
        path: ".omo/evidence/live/output.json".to_string(),
        sha256: h256(b"runtime"),
        size_bytes: 1,
    }])
    .is_err());
}

fn event(value: char) -> String {
    format!("evt_{}", value.to_string().repeat(64))
}

fn hex_char(value: usize) -> char {
    char::from_digit(value as u32, 16).unwrap()
}
