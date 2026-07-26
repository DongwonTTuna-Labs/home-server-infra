mod binding;
mod bootstrap;
mod send;
mod upload;

pub use binding::build_page_binding;
pub use bootstrap::{
    append_accepted, append_allocation, append_claim, append_health_observed, append_health_probe,
    append_host_staged, append_lease, append_owner,
};
pub use send::{
    append_binding, append_materialized, append_model_failed, append_model_started,
    append_model_verified, append_root_failed, append_root_observed, append_root_started,
    append_running, append_send_armed, append_send_clicked, append_send_failed,
    append_send_reconciled, append_send_uncertain, append_upload_completed, append_upload_failed,
    append_upload_started, BindingEvents, SendEventError,
};
pub use upload::{append_upload_cleared, append_upload_mismatch};

use crate::config::now_ms;
use crate::contracts::events::EventEnvelope;

pub fn event_time(predecessor: Option<&EventEnvelope>) -> u64 {
    predecessor.map_or_else(now_ms, |event| now_ms().max(event.created_at_ms))
}
