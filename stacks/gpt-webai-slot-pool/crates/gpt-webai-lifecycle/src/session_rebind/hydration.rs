use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contracts::browser::{EvidenceRef, SessionEcho};
use crate::contracts::ids::{validate_duration_ms, validate_timestamp_ms};

use super::{echo_identity_equal, SessionRebindError, HYDRATION_DEADLINE_MS};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationState {
    LoadingPlaceholder,
    BlankTransient,
    ActiveGenerationVisible,
    AnswerVisible,
    ContentUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HydrationObservation {
    pub sequence_index: u8,
    pub state: HydrationState,
    pub remaining_deadline_ms: u64,
    pub observed_echo: SessionEcho,
    pub evidence_refs: Vec<EvidenceRef>,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HydrationTrace {
    pub observations: Vec<HydrationObservation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HydrationOutcome {
    Running,
    Terminal,
}

impl HydrationObservation {
    pub fn validate(&self) -> Result<(), SessionRebindError> {
        if self.sequence_index >= 50 {
            return Err(SessionRebindError::Invalid("sequenceIndex"));
        }
        validate_duration_ms(self.remaining_deadline_ms)
            .map_err(|_| SessionRebindError::Invalid("remainingDeadlineMs"))?;
        if self.remaining_deadline_ms > HYDRATION_DEADLINE_MS {
            return Err(SessionRebindError::Invalid("remainingDeadlineMs"));
        }
        self.observed_echo
            .validate()
            .map_err(|_| SessionRebindError::Invalid("observedEcho"))?;
        validate_timestamp_ms(self.observed_at_ms)
            .map_err(|_| SessionRebindError::Invalid("observedAtMs"))?;
        if !(1..=4).contains(&self.evidence_refs.len())
            || self
                .evidence_refs
                .iter()
                .any(|item| item.validate().is_err())
        {
            return Err(SessionRebindError::Invalid("evidenceRefs"));
        }
        let paths = self
            .evidence_refs
            .iter()
            .map(|item| item.path.as_str())
            .collect::<BTreeSet<_>>();
        (paths.len() == self.evidence_refs.len())
            .then_some(())
            .ok_or(SessionRebindError::Invalid("duplicate evidenceRefs"))
    }
}

impl HydrationTrace {
    pub fn validate(
        &self,
        rebound_echo: &SessionEcho,
    ) -> Result<HydrationOutcome, SessionRebindError> {
        if !(1..=50).contains(&self.observations.len()) {
            return Err(SessionRebindError::Invalid("hydration observations"));
        }
        let mut previous_time = 0;
        let mut previous_remaining = HYDRATION_DEADLINE_MS;
        for (index, observation) in self.observations.iter().enumerate() {
            observation.validate()?;
            if usize::from(observation.sequence_index) != index
                || observation.observed_at_ms <= previous_time
                || observation.remaining_deadline_ms > previous_remaining
                || !echo_identity_equal(rebound_echo, &observation.observed_echo)
            {
                return Err(SessionRebindError::Invalid("hydration ordering/binding"));
            }
            if index + 1 != self.observations.len()
                && matches!(
                    observation.state,
                    HydrationState::ActiveGenerationVisible
                        | HydrationState::AnswerVisible
                        | HydrationState::ContentUnavailable
                )
            {
                return Err(SessionRebindError::Invalid("hydration early stop"));
            }
            previous_time = observation.observed_at_ms;
            previous_remaining = observation.remaining_deadline_ms;
        }
        let final_observation = self.observations.last().expect("nonempty");
        match final_observation.state {
            HydrationState::ActiveGenerationVisible
                if final_observation.observed_echo.active_turn
                    && final_observation
                        .observed_echo
                        .visible_user_turn_id
                        .is_some()
                    && final_observation
                        .observed_echo
                        .visible_assistant_turn_id
                        .is_some() =>
            {
                Ok(HydrationOutcome::Running)
            }
            HydrationState::AnswerVisible
                if !final_observation.observed_echo.active_turn
                    && final_observation
                        .observed_echo
                        .visible_user_turn_id
                        .is_some()
                    && final_observation
                        .observed_echo
                        .visible_assistant_turn_id
                        .is_some()
                    && final_observation
                        .observed_echo
                        .terminal_answer_sha256
                        .is_some() =>
            {
                Ok(HydrationOutcome::Terminal)
            }
            HydrationState::ContentUnavailable => {
                Err(SessionRebindError::Invalid("session.content_unavailable"))
            }
            _ => Err(SessionRebindError::Invalid("session.hydration_timeout")),
        }
    }
}
