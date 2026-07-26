use crate::locks;
use crate::runtime::control::write_slot_status;
use crate::runtime::RuntimeProbe;

use super::output::{failed_before_lock_output, failed_output, queued_output};
use super::provider_limit::{
    is_provider_limit, reopen_limited_groups_for_retry, ProviderLimitRetryState,
};
use super::release::release_slot;
use super::retry::{
    exhausted_unknown_session, retry_delay_ms, retryable_unknown_session, send_was_attempted,
};
use super::round_trip::run_with_acquired_lease;
use super::selection::select_slot_avoiding_groups;
use super::session::mark_release_failed_by_id;

pub use super::input::RequestRunInput;
pub use super::output::RequestRunOutput;

pub fn run_provider_round_trip(
    input: RequestRunInput,
    runtime: &dyn RuntimeProbe,
) -> RequestRunOutput {
    let mut send_attempts = 0;
    let mut retry_delays_ms = Vec::new();
    let mut slot_reselects = 0;
    let mut provider_limit = ProviderLimitRetryState::default();

    loop {
        let mut output = run_provider_round_trip_once(&input, runtime, &provider_limit);
        if send_was_attempted(&output) {
            send_attempts += 1;
        }
        output = output
            .with_retry_timeline(send_attempts, &retry_delays_ms)
            .with_provider_limit_retry_timeline(provider_limit.retry_delays_ms());

        if is_provider_limit(&output) {
            // Once the provider has established a real session, every recovery path is
            // pinned to that session's slot.  Cohort rotation is only legal before a
            // session exists (for example a visual-gate or send-stage provider limit).
            if output.session_id.is_some() {
                return output;
            }
            provider_limit.record(&output);
            if provider_limit.should_try_another_group_now() {
                continue;
            }
            let Some(delay) = provider_limit.next_cooldown(&input.provider_limit_retry_delays)
            else {
                return output;
            };
            let limited_groups = provider_limit.limited_groups().clone();
            provider_limit.push_cooldown(delay);
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            if let Err(error) =
                reopen_limited_groups_for_retry(&input.config, runtime, &limited_groups)
            {
                return failed_output(&input, None, "provider_limit.retry_reopen_failed", error)
                    .with_retry_timeline(send_attempts, &retry_delays_ms)
                    .with_provider_limit_retry_timeline(provider_limit.retry_delays_ms());
            }
            continue;
        }

        if reselectable_visual_gate_failure(&output)
            && slot_reselects < usize::from(input.config.slot_count)
        {
            slot_reselects += 1;
            continue;
        }
        if reselectable_lock_race(&output) && slot_reselects < usize::from(input.config.slot_count)
        {
            slot_reselects += 1;
            continue;
        }
        if !retryable_unknown_session(&output) {
            return output;
        }
        if input.send_retry_delays.is_empty() {
            return output;
        }
        let Some(delay) = input.send_retry_delays.get(send_attempts - 1).copied() else {
            return exhausted_unknown_session(output)
                .with_retry_timeline(send_attempts, &retry_delays_ms)
                .with_provider_limit_retry_timeline(provider_limit.retry_delays_ms());
        };
        retry_delays_ms.push(retry_delay_ms(delay));
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}

fn run_provider_round_trip_once(
    input: &RequestRunInput,
    runtime: &dyn RuntimeProbe,
    provider_limit: &ProviderLimitRetryState,
) -> RequestRunOutput {
    let decision =
        match select_slot_avoiding_groups(input, runtime, provider_limit.limited_groups()) {
            Ok(Some(decision)) => decision,
            Ok(None) => return queued_output(input),
            Err(error) => return failed_output(input, None, "status.failed", error.to_string()),
        };

    let slot_id = decision.slot_id.0.clone();
    if let Err(error) = locks::acquire_slot_lease(
        &input.config.state_root,
        &slot_id,
        &input.request_id,
        &input.run_id,
        &input.fencing_token,
        input.ttl_ms,
    ) {
        if matches!(error, locks::LockError::Busy(_)) {
            return failed_before_lock_output(
                input,
                Some(&decision),
                "lock.busy",
                error.to_string(),
            );
        }
        return failed_before_lock_output(
            input,
            Some(&decision),
            "lock.acquire_failed",
            error.to_string(),
        );
    }

    let mut output = run_with_acquired_lease(input, &decision, runtime);
    let release = release_slot(input, &slot_id, output.runtime_owned);
    output.lock_released = release.lock_released;
    output.runtime_stopped = release.runtime_stopped;
    output.slot_state_written = release.slot_state_written;
    if let Some(error) = release.error {
        output.ok = false;
        output.status = "release_failed".to_string();
        output.reason = Some(release.reason);
        if let Some(session_id) = output.session_id.clone() {
            mark_release_failed_by_id(input, &session_id, output.reason.as_deref().unwrap());
        }
        output.message = error;
    } else if let Some(status) = post_release_slot_status(&output) {
        match write_slot_status(&input.config, &slot_id, status) {
            Ok(()) => output.slot_state_written = true,
            Err(error) => {
                output.ok = false;
                output.status = "failed".to_string();
                output.reason = Some("slot.state_write_failed".to_string());
                output.message = error.to_string();
            }
        }
    }
    output
}

fn reselectable_visual_gate_failure(output: &RequestRunOutput) -> bool {
    output.lock_released
        && output.session_id.is_none()
        && output.send_status.is_none()
        && visual_gate_slot_status(output).is_some()
}

fn reselectable_lock_race(output: &RequestRunOutput) -> bool {
    output.reason.as_deref() == Some("lock.busy")
        && !output.lock_acquired
        && output.session_id.is_none()
        && output.send_status.is_none()
}

fn visual_gate_slot_status(output: &RequestRunOutput) -> Option<&'static str> {
    if output.reason.as_deref() != Some("visual_gate.failed") {
        return None;
    }
    if output.message.contains("subscription_required")
        || output.message.contains("pro_required")
        || output.message.contains("auth.needs_pro")
    {
        return Some("auth.needs_pro");
    }
    if output.message.contains("provider_limit") || output.message.contains("provider.limit") {
        return Some("provider.limit");
    }
    None
}

fn post_release_slot_status(output: &RequestRunOutput) -> Option<&'static str> {
    if output.reason.as_deref() == Some("provider.limit")
        || output.poll_status.as_deref() == Some("provider_limit")
        || output.send_status.as_deref() == Some("provider_limit")
    {
        return Some("provider.limit");
    }
    visual_gate_slot_status(output)
}
