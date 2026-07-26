use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::contracts::browser::SessionEcho;
use crate::contracts::ids::{
    validate_artifact_claim_id, validate_h256, validate_operation_id, validate_safe_rel_path,
    validate_session_id, validate_turn_id,
};

use super::baseline::ArtifactBaseline;
use super::completion::{reopen_and_verify, PlaywrightDownloadReceipt};
use super::{
    ArtifactClaimError, ArtifactControl, ArtifactExpectation, BottomProof, ZeroControlProof,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Pending,
    ZeroControlsOptionalSuccess,
    Downloaded,
    Failed(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumedAttempt {
    pub attempt_id: String,
    pub control_index: u8,
    pub control: ArtifactControl,
    pub baseline: ArtifactBaseline,
}

#[derive(Clone, Debug)]
pub struct ArtifactClaim {
    pub artifact_claim_id: String,
    pub session_id: String,
    pub terminal_assistant_turn_id: String,
    pub expectation: ArtifactExpectation,
    controls: Vec<ArtifactControl>,
    completed: BTreeMap<u8, PlaywrightDownloadReceipt>,
    consumed: Option<ConsumedAttempt>,
    outcome: ClaimOutcome,
}

impl ArtifactClaim {
    pub fn establish(
        artifact_claim_id: String,
        session_id: String,
        terminal_assistant_turn_id: String,
        expectation: ArtifactExpectation,
    ) -> Result<Self, ArtifactClaimError> {
        validate_artifact_claim_id(&artifact_claim_id)
            .map_err(|_| ArtifactClaimError::Invalid("artifactClaimId"))?;
        validate_session_id(&session_id).map_err(|_| ArtifactClaimError::Invalid("sessionId"))?;
        validate_turn_id(&terminal_assistant_turn_id)
            .map_err(|_| ArtifactClaimError::Invalid("terminalAssistantTurnId"))?;
        Ok(Self {
            artifact_claim_id,
            session_id,
            terminal_assistant_turn_id,
            expectation,
            controls: Vec::new(),
            completed: BTreeMap::new(),
            consumed: None,
            outcome: ClaimOutcome::Pending,
        })
    }

    pub fn discover_controls(
        &mut self,
        controls: Vec<ArtifactControl>,
        bottom_proof: &BottomProof,
    ) -> Result<(), ArtifactClaimError> {
        if self.outcome != ClaimOutcome::Pending
            || !self.controls.is_empty()
            || !(1..=64).contains(&controls.len())
        {
            return Err(ArtifactClaimError::IllegalTransition);
        }
        bottom_proof.validate()?;
        for control in &controls {
            control.validate_for_turn(&self.terminal_assistant_turn_id)?;
        }
        let ids = controls
            .iter()
            .map(|control| control.control_id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != controls.len() {
            return Err(ArtifactClaimError::Invalid("duplicate controls"));
        }
        self.controls = controls;
        Ok(())
    }

    pub fn discover_zero(&mut self, proof: &ZeroControlProof) -> Result<(), ArtifactClaimError> {
        if self.outcome != ClaimOutcome::Pending || !self.controls.is_empty() {
            return Err(ArtifactClaimError::IllegalTransition);
        }
        proof.validate_for(&self.artifact_claim_id, &self.terminal_assistant_turn_id)?;
        self.outcome = match self.expectation {
            ArtifactExpectation::None | ArtifactExpectation::Optional => {
                ClaimOutcome::ZeroControlsOptionalSuccess
            }
            ArtifactExpectation::Required | ArtifactExpectation::Claimed => {
                ClaimOutcome::Failed("artifact.required_zero")
            }
        };
        Ok(())
    }

    pub fn consume_next(
        &mut self,
        attempt_id: String,
        baseline: ArtifactBaseline,
    ) -> Result<&ConsumedAttempt, ArtifactClaimError> {
        if self.outcome != ClaimOutcome::Pending || self.consumed.is_some() {
            return Err(ArtifactClaimError::IllegalTransition);
        }
        validate_operation_id(&attempt_id).map_err(|_| ArtifactClaimError::Invalid("attemptId"))?;
        baseline.validate()?;
        let index = self.completed.len();
        let control = self
            .controls
            .get(index)
            .ok_or(ArtifactClaimError::IllegalTransition)?
            .clone();
        self.consumed = Some(ConsumedAttempt {
            attempt_id,
            control_index: index as u8,
            control,
            baseline,
        });
        Ok(self.consumed.as_ref().expect("set"))
    }

    pub fn complete_consumed(
        &mut self,
        expected: &SessionEcho,
        receipt: PlaywrightDownloadReceipt,
        state_root: &Path,
    ) -> Result<(), ArtifactClaimError> {
        let consumed = self
            .consumed
            .as_ref()
            .ok_or(ArtifactClaimError::IllegalTransition)?;
        receipt.validate_for(
            expected,
            &self.artifact_claim_id,
            &consumed.control,
            &self.terminal_assistant_turn_id,
        )?;
        reopen_and_verify(state_root, &receipt)?;
        self.completed.insert(consumed.control_index, receipt);
        self.consumed = None;
        if self.completed.len() == self.controls.len() {
            self.outcome = ClaimOutcome::Downloaded;
        }
        Ok(())
    }

    pub fn observe_recovery_candidate(
        &self,
        candidate_rel_path: &str,
        sha256: &str,
        stable_observations: u8,
    ) -> Result<(), ArtifactClaimError> {
        if self.consumed.is_none() || stable_observations != 2 {
            return Err(ArtifactClaimError::IllegalTransition);
        }
        validate_safe_rel_path(candidate_rel_path)
            .map_err(|_| ArtifactClaimError::Invalid("candidateRelPath"))?;
        validate_h256(sha256).map_err(|_| ArtifactClaimError::Invalid("sha256"))
    }

    pub fn fail(&mut self, reason: &'static str) -> Result<(), ArtifactClaimError> {
        if self.outcome != ClaimOutcome::Pending {
            return Err(ArtifactClaimError::IllegalTransition);
        }
        self.outcome = ClaimOutcome::Failed(reason);
        Ok(())
    }

    pub fn outcome(&self) -> &ClaimOutcome {
        &self.outcome
    }

    pub fn consumed(&self) -> Option<&ConsumedAttempt> {
        self.consumed.as_ref()
    }
}
