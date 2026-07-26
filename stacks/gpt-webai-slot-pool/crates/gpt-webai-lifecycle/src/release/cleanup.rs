use std::collections::BTreeSet;

use super::{
    EvidenceManifest, ReleaseError, ReleaseFinalStatus, ReleaseStart, ResourceKind, RuntimeOutcome,
};

#[derive(Clone, Debug)]
pub struct ReleaseMachine {
    pub start: ReleaseStart,
    acquired: BTreeSet<ResourceKind>,
    released: BTreeSet<ResourceKind>,
    evidence: Option<EvidenceManifest>,
    runtime_outcome: RuntimeOutcome,
    cleanup_started: bool,
    cleanup_committed: bool,
    standby_written: bool,
    final_status: Option<ReleaseFinalStatus>,
    finalized_at_ms: Option<u64>,
}

impl ReleaseMachine {
    pub fn start(
        start: ReleaseStart,
        acquired: impl IntoIterator<Item = ResourceKind>,
    ) -> Result<Self, ReleaseError> {
        start.validate()?;
        let acquired = acquired.into_iter().collect::<BTreeSet<_>>();
        if acquired.contains(&ResourceKind::RuntimeOwner)
            && !acquired.contains(&ResourceKind::SlotLease)
        {
            return Err(ReleaseError::Invalid("runtime owner without lease"));
        }
        Ok(Self {
            start,
            acquired,
            released: BTreeSet::new(),
            evidence: None,
            runtime_outcome: RuntimeOutcome::Pending,
            cleanup_started: false,
            cleanup_committed: false,
            standby_written: false,
            final_status: None,
            finalized_at_ms: None,
        })
    }

    pub fn preserve_evidence(&mut self, evidence: EvidenceManifest) -> Result<(), ReleaseError> {
        evidence.validate()?;
        if self.evidence.is_some() || self.cleanup_started {
            return Err(ReleaseError::IllegalTransition);
        }
        self.evidence = Some(evidence);
        Ok(())
    }

    pub fn record_runtime_outcome(&mut self, outcome: RuntimeOutcome) -> Result<(), ReleaseError> {
        if self.evidence.is_none()
            || self.runtime_outcome != RuntimeOutcome::Pending
            || outcome == RuntimeOutcome::Pending
        {
            return Err(ReleaseError::IllegalTransition);
        }
        let acquired_owner = self.acquired.contains(&ResourceKind::RuntimeOwner);
        let compatible = match outcome {
            RuntimeOutcome::SkippedNotAcquired => !acquired_owner,
            RuntimeOutcome::Stopped
            | RuntimeOutcome::SkippedOwnerAlive
            | RuntimeOutcome::Failed => acquired_owner,
            RuntimeOutcome::Pending => false,
        };
        if !compatible {
            return Err(ReleaseError::Invalid("runtime outcome/acquisition"));
        }
        self.runtime_outcome = outcome;
        Ok(())
    }

    pub fn start_cleanup(&mut self) -> Result<(), ReleaseError> {
        if self.runtime_outcome == RuntimeOutcome::Pending || self.cleanup_started {
            return Err(ReleaseError::IllegalTransition);
        }
        self.cleanup_started = true;
        Ok(())
    }

    pub fn release_resource(&mut self, resource: ResourceKind) -> Result<(), ReleaseError> {
        if !self.cleanup_started
            || self.cleanup_committed
            || !self.acquired.contains(&resource)
            || !self.released.insert(resource)
        {
            return Err(ReleaseError::IllegalTransition);
        }
        Ok(())
    }

    pub fn commit_cleanup(&mut self) -> Result<(), ReleaseError> {
        if !self.cleanup_started || self.cleanup_committed || self.released != self.acquired {
            return Err(ReleaseError::IllegalTransition);
        }
        self.cleanup_committed = true;
        Ok(())
    }

    pub fn write_standby(&mut self, allocatable: bool) -> Result<(), ReleaseError> {
        if !self.cleanup_committed
            || !self.acquired.contains(&ResourceKind::SlotLease)
            || self.standby_written
            || (self.runtime_outcome == RuntimeOutcome::Failed && allocatable)
        {
            return Err(ReleaseError::IllegalTransition);
        }
        self.standby_written = true;
        Ok(())
    }

    pub fn finalize(
        &mut self,
        requested: ReleaseFinalStatus,
        allocatable: bool,
        finalized_at_ms: u64,
    ) -> Result<(), ReleaseError> {
        if !self.cleanup_committed || self.final_status.is_some() || finalized_at_ms == 0 {
            return Err(ReleaseError::IllegalTransition);
        }
        let has_slot = self.acquired.contains(&ResourceKind::SlotLease);
        let expected = if self.runtime_outcome == RuntimeOutcome::Failed {
            ReleaseFinalStatus::CleanupFailed
        } else if self.runtime_outcome == RuntimeOutcome::SkippedOwnerAlive {
            ReleaseFinalStatus::StopSkippedOwnerAlive
        } else if !has_slot {
            ReleaseFinalStatus::ResourcesReleasedNoSlot
        } else {
            requested
        };
        let valid = requested == expected
            && allocatable == (requested == ReleaseFinalStatus::Allocatable)
            && (!has_slot || self.standby_written)
            && (has_slot || requested == ReleaseFinalStatus::ResourcesReleasedNoSlot);
        if !valid {
            return Err(ReleaseError::Invalid("final status"));
        }
        self.final_status = Some(requested);
        self.finalized_at_ms = Some(finalized_at_ms);
        Ok(())
    }

    pub fn clear_cooldown_and_finalize_allocatable(
        &mut self,
        finalized_at_ms: u64,
    ) -> Result<(), ReleaseError> {
        if self.final_status != Some(ReleaseFinalStatus::CooldownBlocked)
            || !self.standby_written
            || finalized_at_ms == 0
        {
            return Err(ReleaseError::IllegalTransition);
        }
        self.final_status = Some(ReleaseFinalStatus::Allocatable);
        self.finalized_at_ms = Some(finalized_at_ms);
        Ok(())
    }

    pub fn final_status(&self) -> Option<ReleaseFinalStatus> {
        self.final_status
    }

    pub fn all_resources_released(&self) -> bool {
        self.released == self.acquired
    }
}
