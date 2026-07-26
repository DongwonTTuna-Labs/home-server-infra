use serde::Serialize;

use crate::sessions::SessionRecord;
use crate::slots::AllocationDecision;

use super::input::RequestRunInput;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRunOutput {
    pub schema: String,
    pub ok: bool,
    pub status: String,
    pub reason: Option<String>,
    pub request_id: String,
    pub run_id: String,
    pub slot_id: Option<String>,
    pub account_group: Option<String>,
    pub preferred_group: Option<String>,
    pub session_id: Option<String>,
    pub conversation_url: Option<String>,
    pub lock_acquired: bool,
    pub lock_released: bool,
    pub runtime_started: bool,
    pub runtime_owned: bool,
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
    pub send_status: Option<String>,
    pub poll_status: Option<String>,
    pub download_status: Option<String>,
    pub send_attempts: usize,
    pub send_retry_delays_ms: Vec<u64>,
    pub provider_limit_retry_delays_ms: Vec<u64>,
    pub artifacts: usize,
    pub artifact_candidates: usize,
    pub answer_text_len: Option<usize>,
    pub message: String,
}

impl RequestRunOutput {
    pub(crate) fn with_send_status(mut self, status: Option<String>) -> Self {
        self.send_status = status;
        self
    }

    pub(crate) fn with_poll_status(mut self, status: Option<String>) -> Self {
        self.poll_status = status;
        self
    }

    pub(crate) fn with_download_status(mut self, status: Option<String>) -> Self {
        self.download_status = status;
        self
    }

    pub(crate) fn with_runtime_started(mut self, runtime_started: bool) -> Self {
        self.runtime_started = runtime_started;
        self
    }

    pub(crate) fn with_runtime_owned(mut self, runtime_owned: bool) -> Self {
        self.runtime_owned = runtime_owned;
        self
    }

    pub(crate) fn with_artifact_counts(
        mut self,
        artifacts: usize,
        artifact_candidates: usize,
    ) -> Self {
        self.artifacts = artifacts;
        self.artifact_candidates = artifact_candidates;
        self
    }

    pub(crate) fn with_answer_text_len(mut self, answer_text_len: usize) -> Self {
        self.answer_text_len = Some(answer_text_len);
        self
    }

    pub(crate) fn with_session(mut self, session: &SessionRecord) -> Self {
        self.session_id = Some(session.session_id.clone());
        self.conversation_url = Some(session.conversation_url.clone());
        self
    }

    pub(crate) fn with_session_start(mut self, session_id: &str, conversation_url: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self.conversation_url = Some(conversation_url.to_string());
        self
    }

    pub(crate) fn with_session_evidence(
        mut self,
        session_id: Option<String>,
        conversation_url: Option<String>,
    ) -> Self {
        self.session_id = session_id;
        self.conversation_url = conversation_url;
        self
    }

    pub(crate) fn with_retry_timeline(
        mut self,
        send_attempts: usize,
        send_retry_delays_ms: &[u64],
    ) -> Self {
        self.send_attempts = send_attempts;
        self.send_retry_delays_ms = send_retry_delays_ms.to_vec();
        self
    }

    pub(crate) fn with_provider_limit_retry_timeline(
        mut self,
        provider_limit_retry_delays_ms: &[u64],
    ) -> Self {
        self.provider_limit_retry_delays_ms = provider_limit_retry_delays_ms.to_vec();
        self
    }
}

pub(crate) fn queued_output(input: &RequestRunInput) -> RequestRunOutput {
    RequestRunOutput {
        schema: schema(),
        ok: true,
        status: "queued".to_string(),
        reason: Some("slot.pool_busy".to_string()),
        request_id: input.request_id.clone(),
        run_id: input.run_id.clone(),
        slot_id: None,
        account_group: None,
        preferred_group: None,
        session_id: None,
        conversation_url: None,
        lock_acquired: false,
        lock_released: false,
        runtime_started: false,
        runtime_owned: false,
        runtime_stopped: false,
        slot_state_written: false,
        send_status: None,
        poll_status: None,
        download_status: None,
        send_attempts: 0,
        send_retry_delays_ms: Vec::new(),
        provider_limit_retry_delays_ms: Vec::new(),
        artifacts: 0,
        artifact_candidates: 0,
        answer_text_len: None,
        message: "no allocatable provider-ready slot".to_string(),
    }
}

pub(crate) fn failed_output(
    input: &RequestRunInput,
    decision: Option<&AllocationDecision>,
    reason: &str,
    message: String,
) -> RequestRunOutput {
    RequestRunOutput {
        schema: schema(),
        ok: false,
        status: "failed".to_string(),
        reason: Some(reason.to_string()),
        request_id: input.request_id.clone(),
        run_id: input.run_id.clone(),
        slot_id: decision.map(|decision| decision.slot_id.0.clone()),
        account_group: decision.map(|decision| decision.allocated_group.0.clone()),
        preferred_group: decision.map(|decision| decision.preferred_group.0.clone()),
        session_id: None,
        conversation_url: None,
        lock_acquired: decision.is_some(),
        lock_released: false,
        runtime_started: false,
        runtime_owned: false,
        runtime_stopped: false,
        slot_state_written: false,
        send_status: None,
        poll_status: None,
        download_status: None,
        send_attempts: 0,
        send_retry_delays_ms: Vec::new(),
        provider_limit_retry_delays_ms: Vec::new(),
        artifacts: 0,
        artifact_candidates: 0,
        answer_text_len: None,
        message,
    }
}

pub(crate) fn failed_before_lock_output(
    input: &RequestRunInput,
    decision: Option<&AllocationDecision>,
    reason: &str,
    message: String,
) -> RequestRunOutput {
    let mut output = failed_output(input, decision, reason, message);
    output.lock_acquired = false;
    output
}

pub(crate) fn schema() -> String {
    "gpt-webai.lifecycle.run.v2".to_string()
}
