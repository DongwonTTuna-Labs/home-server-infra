use serde::Serialize;

use crate::session_ops::runtime::{SessionRuntimeRelease, SessionRuntimeStart};
use crate::sessions::SessionRecord;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeOutput {
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
    pub answer_text_len: Option<usize>,
    pub runtime_started: bool,
    pub runtime_owned: bool,
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
    pub message: String,
}

pub(super) fn success(
    record: SessionRecord,
    provider_status: String,
    answer_text_len: Option<usize>,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
) -> ResumeOutput {
    ResumeOutput {
        schema: schema(),
        ok: true,
        status: provider_status.clone(),
        reason: None,
        session_id: Some(record.session_id),
        request_id: record.request_id,
        run_id: record.run_id,
        slot_id: Some(record.slot_id),
        account_group: Some(record.cohort),
        conversation_url: Some(record.conversation_url),
        provider_status: Some(provider_status),
        answer_text_len,
        runtime_started: runtime_start.runtime_started,
        runtime_owned: runtime_start.runtime_owned,
        runtime_stopped: runtime_release.runtime_stopped,
        slot_state_written: runtime_release.slot_state_written,
        message: "resumed pinned session through provider".to_string(),
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
) -> ResumeOutput {
    ResumeOutput {
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
        answer_text_len: None,
        runtime_started: runtime_start.runtime_started,
        runtime_owned: runtime_start.runtime_owned,
        runtime_stopped: runtime_release.runtime_stopped,
        slot_state_written: runtime_release.slot_state_written,
        message,
    }
}

fn schema() -> String {
    "gpt-webai.lifecycle.resume.v2".to_string()
}
