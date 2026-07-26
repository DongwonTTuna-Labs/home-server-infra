use std::io;

use thiserror::Error;

use crate::artifact_objects::{write_provider_artifact_objects, ArtifactObjectContext};
use crate::provider_client::ProviderInvocationError;
use crate::sessions::SessionRecordError;

use super::artifacts::TerminalRunContext;
use super::provider::provider_download;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactRecovery {
    pub(crate) download_status: Option<String>,
    pub(crate) artifacts: usize,
    pub(crate) artifact_candidates: usize,
}

#[derive(Debug, Error)]
pub(crate) enum ArtifactRecoveryError {
    #[error("provider artifact download failed: {0}")]
    Provider(#[from] ProviderInvocationError),
    #[error("provider download returned session mismatch")]
    SessionMismatch,
    #[error("provider download status is not done: {0}")]
    NotDone(String),
    #[error("required artifact controls were absent from terminal answer")]
    MissingRequiredControls,
    #[error("provider download saved no files for visible artifact candidates")]
    MissingSavedArtifacts,
    #[error("artifact manifest write failed: {0}")]
    ArtifactManifest(io::Error),
    #[error("persisted session request binding is missing: {0}")]
    RequestBinding(SessionRecordError),
}

pub(crate) fn artifact_failure_reason(error: &ArtifactRecoveryError) -> &'static str {
    match error {
        ArtifactRecoveryError::MissingRequiredControls => "artifact.controls_absent",
        _ => "artifact.recovery_failed",
    }
}

pub(crate) fn recover_artifacts(
    context: &TerminalRunContext<'_>,
) -> Result<ArtifactRecovery, (ArtifactRecoveryError, ArtifactRecovery)> {
    let artifact_candidates = context.poll_summary.artifact_candidates;
    let skipped = ArtifactRecovery {
        download_status: None,
        artifacts: context.poll_summary.artifacts,
        artifact_candidates,
    };
    if artifact_candidates == 0 && context.poll_summary.artifacts == 0 {
        if context
            .input
            .artifact_expectation
            .requires_download_controls()
        {
            return Err((ArtifactRecoveryError::MissingRequiredControls, skipped));
        }
        return Ok(skipped);
    }
    if !context.input.download_artifacts_after_poll {
        return Ok(skipped);
    }

    let download = provider_download(
        context.input,
        context.provider_command,
        &context.session.session_id,
    )
    .map_err(|error| (error.into(), skipped.clone()))?;
    let (request_id, run_id) = context.session.request_binding().map_err(|error| {
        (
            ArtifactRecoveryError::RequestBinding(error),
            skipped.clone(),
        )
    })?;
    write_provider_artifact_objects(
        ArtifactObjectContext {
            config: &context.input.config,
            path_mode: &context.provider_command.path_mode,
            request_id,
            run_id,
            session_id: &context.session.session_id,
            conversation_url: &context.session.conversation_url,
            slot_id: &context.session.slot_id,
            account_group: &context.session.cohort,
        },
        &download.value,
    )
    .map_err(|error| {
        (
            ArtifactRecoveryError::ArtifactManifest(error),
            skipped.clone(),
        )
    })?;
    let recovery = ArtifactRecovery {
        download_status: Some(download.summary.status.clone()),
        artifacts: download.summary.artifacts,
        artifact_candidates,
    };
    if download.summary.session_id.as_deref() != Some(context.session.session_id.as_str()) {
        return Err((ArtifactRecoveryError::SessionMismatch, recovery));
    }
    if download.summary.status != "done" {
        return Err((
            ArtifactRecoveryError::NotDone(download.summary.status),
            recovery,
        ));
    }
    if download.summary.artifacts == 0 {
        return Err((ArtifactRecoveryError::MissingSavedArtifacts, recovery));
    }
    Ok(recovery)
}
