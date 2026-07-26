use std::time::Duration;

use gpt_webai_lifecycle::request::retry::default_send_retry_delays;

#[test]
fn default_send_retry_delays_are_one_three_five_ten_fifteen_seconds() {
    let seconds = default_send_retry_delays()
        .iter()
        .map(Duration::as_secs)
        .collect::<Vec<_>>();

    assert_eq!(seconds, vec![1, 3, 5, 10, 15]);
}
