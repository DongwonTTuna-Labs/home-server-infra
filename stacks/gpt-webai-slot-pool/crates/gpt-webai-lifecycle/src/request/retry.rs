use std::time::Duration;

use super::output::RequestRunOutput;

const DEFAULT_SEND_RETRY_DELAY_SECS: [u64; 5] = [1, 3, 5, 10, 15];

pub fn default_send_retry_delays() -> Vec<Duration> {
    DEFAULT_SEND_RETRY_DELAY_SECS
        .iter()
        .map(|seconds| Duration::from_secs(*seconds))
        .collect()
}

pub(crate) fn retryable_unknown_session(output: &RequestRunOutput) -> bool {
    !output.ok
        && output.reason.as_deref() == Some("provider.send_failed")
        && output.session_id.is_none()
        && output.conversation_url.is_none()
}

pub(crate) fn send_was_attempted(output: &RequestRunOutput) -> bool {
    output.send_status.is_some()
        || output.session_id.is_some()
        || matches!(
            output.reason.as_deref(),
            Some("provider.send_failed" | "send.confirmation_failed")
        )
        || output.status == "done"
}

pub(crate) fn retry_delay_ms(delay: Duration) -> u64 {
    delay.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn exhausted_unknown_session(mut output: RequestRunOutput) -> RequestRunOutput {
    output.ok = false;
    output.status = "failed".to_string();
    output.reason = Some("send.unknown_session".to_string());
    output.message =
        "provider send never produced a confirmed sessionId after retry budget".to_string();
    output
}
