use std::time::Duration;

use crate::request::provider_limit::default_provider_limit_retry_delays;
use crate::request::retry::default_send_retry_delays;

pub(super) fn send_retry_delays(fake_mode: bool) -> Vec<Duration> {
    if fake_mode {
        Vec::new()
    } else {
        default_send_retry_delays()
    }
}

pub(super) fn provider_limit_retry_delays(fake_mode: bool) -> Vec<Duration> {
    if fake_mode {
        Vec::new()
    } else {
        default_provider_limit_retry_delays()
    }
}
