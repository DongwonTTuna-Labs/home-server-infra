use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROVIDER_LIMIT_STATUS: &str = "provider.limit";
const OBSERVED_AT_KEY: &str = "provider_limit_observed_at_ms";
const NEXT_RETRY_AT_KEY: &str = "provider_limit_next_retry_at_ms";
const RECHECK_DELAY: Duration = Duration::from_secs(180);

pub(crate) fn slot_state_body(status: &str) -> String {
    if status != PROVIDER_LIMIT_STATUS {
        return format!("status={status}\n");
    }
    let observed_at = epoch_ms(crate::config::now_system_time());
    let next_retry_at = observed_at.saturating_add(duration_ms(RECHECK_DELAY));
    format!(
        "status={status}\n{OBSERVED_AT_KEY}={observed_at}\n{NEXT_RETRY_AT_KEY}={next_retry_at}\n"
    )
}

pub(crate) fn recheck_due(
    values: &BTreeMap<String, String>,
    state_file: &Path,
    now: SystemTime,
) -> bool {
    if values.get("status").map(String::as_str) != Some(PROVIDER_LIMIT_STATUS) {
        return false;
    }
    let now_ms = epoch_ms(now);
    if let Some(next_retry_at) = values
        .get(NEXT_RETRY_AT_KEY)
        .and_then(|value| value.parse::<u64>().ok())
    {
        return now_ms >= next_retry_at;
    }
    modified_at(state_file)
        .map(|modified| now_ms >= epoch_ms(modified).saturating_add(duration_ms(RECHECK_DELAY)))
        .unwrap_or(false)
}

fn modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
}

fn epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(duration_ms)
        .unwrap_or(0)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
