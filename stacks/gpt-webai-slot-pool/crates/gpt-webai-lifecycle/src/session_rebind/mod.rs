pub mod hydration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::browser::{SessionEcho, SessionRebindExpectation};
use crate::contracts::ids::{
    validate_byte_count, validate_h256, validate_safe_rel_path, validate_turn_id,
};

use self::hydration::{HydrationOutcome, HydrationTrace};

pub const NAVIGATION_ATTEMPT_LIMIT: u8 = 2;
pub const NAVIGATION_ATTEMPT_TIMEOUT_MS: u64 = 30_000;
pub const HYDRATION_DEADLINE_MS: u64 = 90_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TerminalAnswerObservation {
    pub answer_rel_path: String,
    pub answer_sha256: String,
    pub answer_size_bytes: u64,
    pub terminal_assistant_turn_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebindProof {
    pub expectation: SessionRebindExpectation,
    pub observed_echo: SessionEcho,
    pub page_binding_generation: u16,
    pub hydration: HydrationTrace,
    pub terminal_answer: Option<TerminalAnswerObservation>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SessionRebindError {
    #[error("invalid session rebind field: {0}")]
    Invalid(&'static str),
}

impl TerminalAnswerObservation {
    pub fn validate(&self) -> Result<(), SessionRebindError> {
        valid(
            validate_safe_rel_path(&self.answer_rel_path),
            "answerRelPath",
        )?;
        valid(validate_h256(&self.answer_sha256), "answerSha256")?;
        valid(
            validate_byte_count(self.answer_size_bytes),
            "answerSizeBytes",
        )?;
        valid(
            validate_turn_id(&self.terminal_assistant_turn_id),
            "terminalAssistantTurnId",
        )
    }
}

impl RebindProof {
    pub fn validate(
        &self,
        requested_expectation: &SessionRebindExpectation,
    ) -> Result<HydrationOutcome, SessionRebindError> {
        requested_expectation
            .validate()
            .map_err(|_| SessionRebindError::Invalid("expectation"))?;
        if &self.expectation != requested_expectation {
            return Err(SessionRebindError::Invalid("expectation echo"));
        }
        validate_observed_echo(requested_expectation, &self.observed_echo)?;
        let expected_generation = requested_expectation
            .last_known_page_binding_generation
            .checked_add(1)
            .ok_or(SessionRebindError::Invalid(
                "pageBindingGeneration overflow",
            ))?;
        if self.page_binding_generation != expected_generation
            || self.observed_echo.page_binding_generation != expected_generation
        {
            return Err(SessionRebindError::Invalid("pageBindingGeneration"));
        }
        let outcome = self.hydration.validate(&self.observed_echo)?;
        match (&outcome, &self.terminal_answer) {
            (HydrationOutcome::Terminal, Some(answer)) => {
                answer.validate()?;
                let final_echo = &self
                    .hydration
                    .observations
                    .last()
                    .expect("validated")
                    .observed_echo;
                if final_echo.terminal_answer_sha256.as_deref()
                    != Some(answer.answer_sha256.as_str())
                    || final_echo.visible_assistant_turn_id.as_deref()
                        != Some(answer.terminal_assistant_turn_id.as_str())
                {
                    return Err(SessionRebindError::Invalid("terminalAnswer binding"));
                }
            }
            (HydrationOutcome::Running, None) => {}
            _ => return Err(SessionRebindError::Invalid("terminalAnswer nullability")),
        }
        Ok(outcome)
    }
}

pub fn validate_observed_echo(
    expected: &SessionRebindExpectation,
    observed: &SessionEcho,
) -> Result<(), SessionRebindError> {
    observed
        .validate()
        .map_err(|_| SessionRebindError::Invalid("observedEcho"))?;
    let page = &observed.page_binding;
    let required = observed.session_id == expected.session_id
        && observed.conversation_url == expected.conversation_url
        && page.slot_id == expected.slot_id
        && page.cohort == expected.cohort
        && page.lease_id == expected.lease_id
        && page.lease_generation == expected.lease_generation
        && page.runtime_owner_id == expected.runtime_owner_id
        && page.runtime_owner_generation == expected.runtime_owner_generation
        && page.runtime_incarnation_id == expected.runtime_incarnation_id
        && expected
            .request_id
            .as_ref()
            .is_none_or(|value| observed.request_id.as_ref() == Some(value))
        && expected
            .run_id
            .as_ref()
            .is_none_or(|value| observed.run_id.as_ref() == Some(value));
    required
        .then_some(())
        .ok_or(SessionRebindError::Invalid("observedEcho identity"))
}

pub(crate) fn echo_identity_equal(first: &SessionEcho, second: &SessionEcho) -> bool {
    first.page_binding == second.page_binding
        && first.session_id == second.session_id
        && first.conversation_url == second.conversation_url
        && first.request_id == second.request_id
        && first.run_id == second.run_id
        && first.session_binding_id == second.session_binding_id
        && first.page_binding_generation == second.page_binding_generation
}

fn valid<T, E>(result: Result<T, E>, field: &'static str) -> Result<(), SessionRebindError> {
    result
        .map(|_| ())
        .map_err(|_| SessionRebindError::Invalid(field))
}
