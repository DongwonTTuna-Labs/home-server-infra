use serde::Serialize;

use crate::runtime::control::RuntimeReleaseResult;

use super::target::TargetError;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReleaseOutput {
    schema: String,
    ok: bool,
    status: String,
    reason: Option<String>,
    session_id: Option<String>,
    slot_id: Option<String>,
    lock_released: bool,
    runtime_stopped: bool,
    slot_state_written: bool,
    message: String,
}

pub(super) fn released(
    session_id: Option<String>,
    slot_id: String,
    lock_released: bool,
    reason: Option<&str>,
    message: &str,
    runtime: RuntimeReleaseResult,
) -> ReleaseOutput {
    ReleaseOutput {
        schema: schema(),
        ok: true,
        status: "released".to_string(),
        reason: reason.map(str::to_string),
        session_id,
        slot_id: Some(slot_id),
        lock_released,
        runtime_stopped: runtime.runtime_stopped,
        slot_state_written: runtime.slot_state_written,
        message: message.to_string(),
    }
}

pub(super) fn failed(
    session_id: Option<String>,
    slot_id: Option<String>,
    reason: &str,
    message: String,
    runtime: RuntimeReleaseResult,
) -> ReleaseOutput {
    ReleaseOutput {
        schema: schema(),
        ok: false,
        status: "failed".to_string(),
        reason: Some(reason.to_string()),
        session_id,
        slot_id,
        lock_released: false,
        runtime_stopped: runtime.runtime_stopped,
        slot_state_written: runtime.slot_state_written,
        message,
    }
}

pub(super) fn target_failed(error: TargetError) -> ReleaseOutput {
    failed(
        Some(error.session_id),
        None,
        error.reason,
        error.message,
        RuntimeReleaseResult {
            runtime_stopped: false,
            slot_state_written: false,
        },
    )
}

fn schema() -> String {
    "gpt-webai.lifecycle.release.v2".to_string()
}
