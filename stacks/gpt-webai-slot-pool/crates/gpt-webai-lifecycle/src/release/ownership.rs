use crate::contracts::events::Writer;
use crate::contracts::projection::RuntimeOwnerRecord;
use crate::runtime::ownership::{
    current_owner_can_stop, takeover, validate_dead_owner, DeadOwnerProof,
};

use super::ReleaseError;

#[derive(Clone, Debug)]
pub enum StopAuthorization {
    NotAcquired,
    CurrentOwner(RuntimeOwnerRecord),
    Takeover {
        retired: Box<RuntimeOwnerRecord>,
        replacement: Box<RuntimeOwnerRecord>,
    },
    OwnerAliveOrUnknown {
        owner: RuntimeOwnerRecord,
        proof_attempt: Option<Box<DeadOwnerProof>>,
    },
}

pub struct StopAuthorizationInput<'a> {
    pub owner: Option<&'a RuntimeOwnerRecord>,
    pub presented_generation: Option<u16>,
    pub fencing_token: Option<&'a str>,
    pub now_ms: u64,
    pub dead_owner_proof: Option<&'a DeadOwnerProof>,
    pub release_id: &'a str,
    pub takeover_writer: Writer,
    pub takeover_event_id: String,
}

pub fn authorize_stop(
    input: StopAuthorizationInput<'_>,
) -> Result<StopAuthorization, ReleaseError> {
    let Some(owner) = input.owner else {
        return Ok(StopAuthorization::NotAcquired);
    };
    match (input.presented_generation, input.fencing_token) {
        (Some(generation), Some(token)) => {
            if current_owner_can_stop(owner, generation, token, input.now_ms) {
                return Ok(StopAuthorization::CurrentOwner(owner.clone()));
            }
            return Err(ReleaseError::FencingMismatch);
        }
        (None, None) => {}
        _ => return Err(ReleaseError::FencingMismatch),
    }
    if let Some(proof) = input.dead_owner_proof {
        if validate_dead_owner(owner, proof).is_ok() {
            let (retired, replacement) = takeover(
                owner,
                proof,
                input.release_id,
                input.takeover_writer,
                input.takeover_event_id,
            )
            .map_err(|error| ReleaseError::Ownership(error.to_string()))?;
            return Ok(StopAuthorization::Takeover {
                retired: Box::new(retired),
                replacement: Box::new(replacement),
            });
        }
    }
    Ok(StopAuthorization::OwnerAliveOrUnknown {
        owner: owner.clone(),
        proof_attempt: input.dead_owner_proof.cloned().map(Box::new),
    })
}

impl StopAuthorization {
    pub fn stop_owner(&self) -> Option<&RuntimeOwnerRecord> {
        match self {
            Self::CurrentOwner(owner) => Some(owner),
            Self::Takeover { replacement, .. } => Some(replacement),
            Self::NotAcquired | Self::OwnerAliveOrUnknown { .. } => None,
        }
    }
}
