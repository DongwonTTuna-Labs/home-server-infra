use gpt_webai_lifecycle::allocator::cursors::advance;
use gpt_webai_lifecycle::allocator::health::{
    map_health, picker_failure_cooldown_ms, status_result_kind,
};
use gpt_webai_lifecycle::allocator::scan::{scan, SkipReason};
use gpt_webai_lifecycle::contracts::health::HealthStatus;
use gpt_webai_lifecycle::contracts::projection::AllocatorRecord;

#[test]
fn zeroed_allocator_reproduces_the_exact_fifteen_grant_trace() {
    let mut allocator = AllocatorRecord::zeroed(event_id('0'));
    let actual = (0..15)
        .map(|ordinal| advance(&mut allocator, ordinal).slot_id)
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        [
            "slot-01", "slot-04", "slot-08", "slot-02", "slot-05", "slot-09", "slot-03", "slot-06",
            "slot-10", "slot-01", "slot-07", "slot-08", "slot-02", "slot-04", "slot-09",
        ]
    );
}

#[test]
fn unavailable_slot_is_observed_before_the_next_grant_without_cursor_rollback() {
    let mut allocator = AllocatorRecord::zeroed(event_id('0'));
    let result = scan(&mut allocator, |slot| {
        Ok((slot == "slot-01").then_some(SkipReason::HealthBlocked))
    })
    .expect("scan");

    assert_eq!(result.granted_slot_id.as_deref(), Some("slot-04"));
    assert_eq!(result.observations.len(), 2);
    assert_eq!(result.observations[0].slot_id, "slot-01");
    assert_eq!(result.observations[0].decision, "skip");
    assert_eq!(
        result.observations[0]
            .skip_reason
            .expect("skip reason")
            .as_str(),
        "health_blocked"
    );
    assert_eq!(result.observations[1].slot_id, "slot-04");
    assert_eq!(result.observations[1].decision, "grantable");
    assert_eq!(allocator.cohort_cursor, 2);
    assert_eq!(allocator.within_cursors.cohort_a, 1);
    assert_eq!(allocator.within_cursors.cohort_b, 1);
    assert_eq!(allocator.within_cursors.cohort_c, 0);

    let next = scan(&mut allocator, |_| Ok(None)).expect("next scan");
    assert_eq!(next.granted_slot_id.as_deref(), Some("slot-08"));
}

#[test]
fn bounded_scan_records_exactly_ten_skips_and_no_grant() {
    let mut allocator = AllocatorRecord::zeroed(event_id('0'));
    let reasons = [
        SkipReason::Leased,
        SkipReason::ClaimActive,
        SkipReason::Cooldown,
        SkipReason::RuntimeOwned,
        SkipReason::HealthBlocked,
        SkipReason::StateInvalid,
    ];
    let result = scan(&mut allocator, |slot| {
        let number = slot.trim_start_matches("slot-").parse::<usize>().unwrap();
        Ok(Some(reasons[(number - 1) % reasons.len()]))
    })
    .expect("bounded scan");

    assert_eq!(result.granted_slot_id, None);
    assert_eq!(result.observations.len(), 10);
    assert_eq!(
        result
            .observations
            .iter()
            .map(|observation| observation.scan_ordinal)
            .collect::<Vec<_>>(),
        (0_u8..10).collect::<Vec<_>>()
    );
    assert!(result
        .observations
        .iter()
        .all(|observation| observation.decision == "skip" && observation.skip_reason.is_some()));
}

#[test]
fn health_mapping_uses_the_closed_cooldown_and_retry_rules() {
    let ready = map_health(HealthStatus::Ready, None);
    assert!(ready.allocatable);
    assert_eq!(ready.cooldown_ms, 0);
    assert_eq!(ready.retry_after_ms, None);

    let provider_low = map_health(HealthStatus::ProviderLimit, Some(1));
    let provider_high = map_health(HealthStatus::ProviderLimit, Some(9_000_000));
    assert_eq!(provider_low.cooldown_ms, 60_000);
    assert_eq!(provider_high.cooldown_ms, 3_600_000);
    assert_eq!(
        map_health(HealthStatus::Unreachable, None).retry_after_ms,
        Some(250)
    );
    assert_eq!(map_health(HealthStatus::Unknown, None).cooldown_ms, 60_000);
    assert_eq!(picker_failure_cooldown_ms(), 300_000);
    assert_eq!(HealthStatus::parse("not-a-health-status"), None);
}

#[test]
fn health_status_to_lifecycle_status_result_is_exhaustive_after_retry() {
    let cases = [
        (HealthStatus::Ready, "status.ready"),
        (HealthStatus::ReadyModelCorrectionRequired, "status.ready"),
        (HealthStatus::LoginRequired, "status.blocked"),
        (HealthStatus::SubscriptionRequired, "status.blocked"),
        (HealthStatus::ProviderLimit, "status.degraded"),
        (HealthStatus::Unreachable, "status.degraded"),
        (HealthStatus::SchemaDrift, "status.degraded"),
        (HealthStatus::Unknown, "status.degraded"),
    ];
    for (health, expected) in cases {
        assert_eq!(status_result_kind(health), expected, "{health}");
        assert_eq!(HealthStatus::parse(health.as_str()), Some(health));
    }
    assert_eq!(HealthStatus::ALL.len(), 8);
}

fn event_id(value: char) -> String {
    format!("evt_{}", value.to_string().repeat(64))
}
