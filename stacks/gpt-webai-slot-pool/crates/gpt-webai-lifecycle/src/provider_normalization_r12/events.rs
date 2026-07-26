use thiserror::Error;

use crate::contracts::events::EventType;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterStage {
    Rebind,
    Poll,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedEventResolution {
    Event(EventType),
    NoEvent,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EventTokenError {
    #[error("unknown R13 event token: {0}")]
    UnknownR13(String),
    #[error("unknown retained event token: {0}")]
    UnknownRetained(String),
    #[error("retained event requires an operation-stage resolution: {0}")]
    StageRequired(String),
    #[error("retained event requires a caller-selected recovery or QA resolution: {0}")]
    ContextRequired(String),
    #[error("event sequence is empty, malformed, or contains an embedded comma")]
    InvalidSequence,
}

pub fn parse_r13_event_sequence(cell: &str) -> Result<Vec<EventType>, EventTokenError> {
    if cell == "-" {
        return Ok(Vec::new());
    }
    if cell.is_empty()
        || cell
            .bytes()
            .any(|byte| matches!(byte, b'\t' | b'\n' | b'\r'))
    {
        return Err(EventTokenError::InvalidSequence);
    }
    cell.split(',')
        .map(|token| {
            if token.is_empty() {
                return Err(EventTokenError::InvalidSequence);
            }
            event_type(token).ok_or_else(|| EventTokenError::UnknownR13(token.to_string()))
        })
        .collect()
}

pub fn translate_retained_event(
    token: &str,
    stage: AdapterStage,
) -> Result<RetainedEventResolution, EventTokenError> {
    let name = token
        .strip_prefix("event.")
        .ok_or_else(|| EventTokenError::UnknownRetained(token.to_string()))?;
    if let Some(prior) = name.strip_prefix("prior.") {
        return translate_prior(prior);
    }
    let event = match name {
        "ProviderPollProgressR12" => EventType::PollProgress,
        "ProviderPollFailedR12" => EventType::PollFailed,
        "ProviderAnswerTerminalR12" => EventType::AnswerTerminal,
        "ProviderSendClickedR12" => EventType::SendClicked,
        "ProviderSendReconciledR12" => EventType::SendReconciled,
        "ProviderSendUncertainR12" => EventType::SendUncertain,
        "ProviderTurnStartConfirmedR12" => EventType::TurnStartConfirmed,
        "ProviderSessionBindingEstablishedR12" => EventType::SessionBindingEstablished,
        "ProviderRunningProjectedR12" => EventType::RunningProjected,
        "ProviderSessionHydrationObservedR12" => EventType::SessionHydrationObserved,
        "ProviderSessionHydratedR12" => EventType::SessionHydrated,
        "ProviderArtifactClaimEstablishedR12" => EventType::ArtifactClaimEstablished,
        "ProviderArtifactControlsDiscoveredR12" => EventType::ArtifactControlsDiscovered,
        "ProviderArtifactControlsAbsentR12" => EventType::ArtifactControlsAbsent,
        "ProviderArtifactDownloadAttemptConsumedR12" => EventType::ArtifactDownloadAttemptConsumed,
        "ProviderArtifactDownloadCompletedR12" => EventType::ArtifactDownloadCompleted,
        "ProviderArtifactClaimCompletedR12" => EventType::ArtifactClaimCompleted,
        "ProviderArtifactClaimFailedR12" => EventType::ArtifactClaimFailed,
        "ProviderTerminalPersistedR12" => EventType::TerminalPersisted,
        "ProviderModelEnsureFailedR12" => EventType::ModelSelectionFailed,
        "ProviderUploadFailedR12" => EventType::UploadFailed,
        "ProviderSlotHealthUpdatedR12" => EventType::SlotHealthObserved,
        "ProviderDownloadObservationR12" => EventType::ArtifactRecoveryCandidateObserved,
        "ProviderSessionUrlRejectedR12" => match stage {
            AdapterStage::Rebind => EventType::SessionOperationFailed,
            AdapterStage::Poll => EventType::PollFailed,
            AdapterStage::Other => {
                return Err(EventTokenError::StageRequired(token.to_string()));
            }
        },
        _ => return Err(EventTokenError::UnknownRetained(token.to_string())),
    };
    Ok(RetainedEventResolution::Event(event))
}

fn translate_prior(name: &str) -> Result<RetainedEventResolution, EventTokenError> {
    let resolution = match name {
        "RequestClaimed" => RetainedEventResolution::Event(EventType::RequestClaimGranted),
        "RuntimeOwnershipIntent" | "SessionRuntimeOwnershipIntent" => {
            RetainedEventResolution::NoEvent
        }
        "RuntimeStarted" => RetainedEventResolution::Event(EventType::RuntimeOwnershipGranted),
        "RuntimeAlreadyOwned" => RetainedEventResolution::Event(EventType::RuntimeOwnershipAdopted),
        "SlotHealthUpdated" => RetainedEventResolution::Event(EventType::SlotHealthObserved),
        "ProvisionalPageBindingEstablished" => {
            RetainedEventResolution::Event(EventType::RootCaptureObserved)
        }
        "ModelEnsureStarted" => RetainedEventResolution::Event(EventType::ModelSelectionStarted),
        "ModelEnsureVerified" => RetainedEventResolution::Event(EventType::ModelSelectionVerified),
        "ModelEnsureFailed" => RetainedEventResolution::Event(EventType::ModelSelectionFailed),
        "SessionOperationClaimed" => {
            RetainedEventResolution::Event(EventType::SessionOperationClaimGranted)
        }
        "SessionRuntimeStarted" => {
            RetainedEventResolution::Event(EventType::SessionRuntimeOwnershipGranted)
        }
        "SessionRuntimeAlreadyOwned" => {
            RetainedEventResolution::Event(EventType::SessionRuntimeOwnershipAdopted)
        }
        "ProjectionRebuilt"
        | "QAUnrelatedBaselineCaptured"
        | "QASourceFingerprintCaptured"
        | "QALiveCycleStarted"
        | "QALiveCycleCompleted"
        | "QAReviewStarted"
        | "QAReviewCompleted" => {
            return Err(EventTokenError::ContextRequired(name.to_string()));
        }
        identical => RetainedEventResolution::Event(
            event_type(identical)
                .ok_or_else(|| EventTokenError::UnknownRetained(name.to_string()))?,
        ),
    };
    Ok(resolution)
}

fn event_type(name: &str) -> Option<EventType> {
    EventType::ALL
        .iter()
        .copied()
        .find(|event| event.as_str() == name)
}
