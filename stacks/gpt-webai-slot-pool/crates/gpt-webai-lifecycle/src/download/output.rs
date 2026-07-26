use serde::Serialize;

use crate::session_ops::runtime::{SessionRuntimeRelease, SessionRuntimeStart};
use crate::sessions::SessionRecord;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOutput {
    pub schema: String,
    pub ok: bool,
    pub status: String,
    pub reason: Option<String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub slot_id: Option<String>,
    pub account_group: Option<String>,
    pub conversation_url: Option<String>,
    pub provider_status: Option<String>,
    pub artifacts: usize,
    pub artifact_candidates: usize,
    pub runtime_started: bool,
    pub runtime_owned: bool,
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
    pub message: String,
}

pub(super) fn success(
    record: SessionRecord,
    provider_status: String,
    provider_reason: Option<String>,
    artifacts: usize,
    artifact_candidates: usize,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
) -> DownloadOutput {
    let ok = provider_status == "done";
    DownloadOutput {
        schema: schema(),
        ok,
        status: provider_status.clone(),
        reason: if ok {
            None
        } else {
            provider_reason.or_else(|| Some(provider_status.clone()))
        },
        session_id: Some(record.session_id),
        request_id: record.request_id,
        run_id: record.run_id,
        slot_id: Some(record.slot_id),
        account_group: Some(record.cohort),
        conversation_url: Some(record.conversation_url),
        provider_status: Some(provider_status),
        artifacts,
        artifact_candidates,
        runtime_started: runtime_start.runtime_started,
        runtime_owned: runtime_start.runtime_owned,
        runtime_stopped: runtime_release.runtime_stopped,
        slot_state_written: runtime_release.slot_state_written,
        message: "downloaded artifacts through pinned session provider".to_string(),
    }
}

pub(super) fn failed(
    session_id: &str,
    record: Option<&SessionRecord>,
    reason: &str,
    message: String,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
    provider_status: Option<String>,
) -> DownloadOutput {
    DownloadOutput {
        schema: schema(),
        ok: false,
        status: "failed".to_string(),
        reason: Some(reason.to_string()),
        session_id: Some(session_id.to_string()),
        request_id: record.and_then(|record| record.request_id.clone()),
        run_id: record.and_then(|record| record.run_id.clone()),
        slot_id: record.map(|record| record.slot_id.clone()),
        account_group: record.map(|record| record.cohort.clone()),
        conversation_url: record.map(|record| record.conversation_url.clone()),
        provider_status,
        artifacts: 0,
        artifact_candidates: 0,
        runtime_started: runtime_start.runtime_started,
        runtime_owned: runtime_start.runtime_owned,
        runtime_stopped: runtime_release.runtime_stopped,
        slot_state_written: runtime_release.slot_state_written,
        message,
    }
}

fn schema() -> String {
    "gpt-webai.lifecycle.download.v2".to_string()
}
