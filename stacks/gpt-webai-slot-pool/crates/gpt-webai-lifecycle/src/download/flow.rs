use thiserror::Error;

use crate::artifact_objects::{write_provider_artifact_objects, ArtifactObjectContext};
use crate::provider_client::{
    run_provider_invocation, ProviderInvocation, ProviderInvocationError, ProviderOperation,
};
use crate::provider_runner::{ProviderCommandContext, ProviderRunnerError};
use crate::runtime::control::RuntimeControlError;
use crate::runtime::RuntimeProbe;
use crate::session_ops::runtime::{
    ensure_session_runtime_started, no_session_runtime_release, no_session_runtime_start,
    stop_owned_session_runtime, SessionRuntimeError, SessionRuntimeRelease,
    SessionRuntimeReleaseInput, SessionRuntimeStart, SessionRuntimeStartInput,
};
use crate::sessions::{read_session_record, SessionRecord, SessionRecordError};

use super::output::{failed, success, DownloadOutput};
use super::DownloadInput;

#[derive(Debug, Error)]
enum DownloadError {
    #[error("session record error: {0}")]
    Session(#[from] SessionRecordError),
    #[error("provider command failed: {0}")]
    ProviderCommand(#[from] ProviderRunnerError),
    #[error("provider invocation failed: {0}")]
    ProviderInvocation(#[from] ProviderInvocationError),
    #[error("runtime start failed: {0}")]
    RuntimeStart(#[from] SessionRuntimeError),
    #[error("runtime stop failed: {0}")]
    RuntimeRelease(#[from] RuntimeControlError),
    #[error("provider returned session mismatch")]
    SessionMismatch,
    #[error("artifact manifest write failed: {0}")]
    ArtifactManifest(std::io::Error),
}

struct DownloadFailure {
    error: DownloadError,
    record: Option<SessionRecord>,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
    provider_status: Option<String>,
}

pub(super) fn run(input: DownloadInput, runtime: &dyn RuntimeProbe) -> DownloadOutput {
    match try_download(&input, runtime) {
        Ok(output) => output,
        Err(failure) => failed(
            &input.session_id,
            failure.record.as_ref(),
            reason_for(&failure.error),
            failure.error.to_string(),
            failure.runtime_start,
            failure.runtime_release,
            failure.provider_status,
        ),
    }
}

fn try_download(
    input: &DownloadInput,
    runtime: &dyn RuntimeProbe,
) -> Result<DownloadOutput, Box<DownloadFailure>> {
    let record = read_session_record(&input.config.state_root, &input.session_id)
        .map_err(|error| failure(DownloadError::from(error), None))?;
    let (request_id, run_id) = record
        .request_binding()
        .map_err(|error| failure(DownloadError::from(error), Some(record.clone())))?;
    let runtime_start = ensure_session_runtime_started(
        SessionRuntimeStartInput {
            config: &input.config,
            slot_id: &record.slot_id,
            mode: &input.runtime_start_mode,
        },
        runtime,
    )
    .map_err(|error| failure(DownloadError::from(error), Some(record.clone())))?;
    let command = input
        .provider_execution
        .command(ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            run_id,
        })
        .map_err(|error| {
            failure_after_runtime_start(input, &record, DownloadError::from(error), &runtime_start)
        })?;
    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Download {
            session_id: record.session_id.clone(),
            artifact_expectation: input.artifact_expectation,
        },
        env: command.env.clone(),
        timeout: input.provider_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
    .map_err(|error| {
        failure_after_runtime_start(input, &record, DownloadError::from(error), &runtime_start)
    })?;
    write_provider_artifact_objects(
        ArtifactObjectContext {
            config: &input.config,
            path_mode: &command.path_mode,
            request_id,
            run_id,
            session_id: &record.session_id,
            conversation_url: &record.conversation_url,
            slot_id: &record.slot_id,
            account_group: &record.cohort,
        },
        &result.value,
    )
    .map_err(|error| {
        failure_after_runtime_start(
            input,
            &record,
            DownloadError::ArtifactManifest(error),
            &runtime_start,
        )
    })?;
    if result.summary.session_id.as_deref() != Some(record.session_id.as_str()) {
        return Err(failure_after_runtime_start(
            input,
            &record,
            DownloadError::SessionMismatch,
            &runtime_start,
        ));
    }
    let runtime_release = stop_owned_session_runtime(
        SessionRuntimeReleaseInput {
            config: &input.config,
            slot_id: &record.slot_id,
            mode: &input.runtime_release_mode,
        },
        runtime_start.runtime_owned,
    )
    .map_err(|error| {
        failure_with_runtime(
            DownloadError::from(error),
            Some(record.clone()),
            runtime_start.clone(),
            no_session_runtime_release(),
            Some(result.summary.status.clone()),
        )
    })?;
    Ok(success(
        record,
        result.summary.status,
        result.summary.reason,
        result.summary.artifacts,
        result.summary.artifact_candidates,
        runtime_start,
        runtime_release,
    ))
}

fn failure(error: DownloadError, record: Option<SessionRecord>) -> Box<DownloadFailure> {
    failure_with_runtime(
        error,
        record,
        no_session_runtime_start(),
        no_session_runtime_release(),
        None,
    )
}

fn failure_after_runtime_start(
    input: &DownloadInput,
    record: &SessionRecord,
    error: DownloadError,
    runtime_start: &SessionRuntimeStart,
) -> Box<DownloadFailure> {
    match stop_owned_session_runtime(
        SessionRuntimeReleaseInput {
            config: &input.config,
            slot_id: &record.slot_id,
            mode: &input.runtime_release_mode,
        },
        runtime_start.runtime_owned,
    ) {
        Ok(release) => failure_with_runtime(
            error,
            Some(record.clone()),
            runtime_start.clone(),
            release,
            None,
        ),
        Err(release_error) => failure_with_runtime(
            DownloadError::from(release_error),
            Some(record.clone()),
            runtime_start.clone(),
            no_session_runtime_release(),
            None,
        ),
    }
}

fn failure_with_runtime(
    error: DownloadError,
    record: Option<SessionRecord>,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
    provider_status: Option<String>,
) -> Box<DownloadFailure> {
    Box::new(DownloadFailure {
        error,
        record,
        runtime_start,
        runtime_release,
        provider_status,
    })
}

fn reason_for(error: &DownloadError) -> &'static str {
    match error {
        DownloadError::Session(SessionRecordError::Missing(_)) => "session.record_missing",
        DownloadError::Session(SessionRecordError::Collision(_))
        | DownloadError::Session(SessionRecordError::Invalid(_))
        | DownloadError::Session(SessionRecordError::InvalidConversationUrl(_))
        | DownloadError::Session(SessionRecordError::Json(_)) => "session.record_invalid",
        DownloadError::Session(SessionRecordError::Io(_)) => "session.record_read_failed",
        DownloadError::ProviderCommand(_) => "provider.command_failed",
        DownloadError::ProviderInvocation(_) => "provider.invocation_failed",
        DownloadError::RuntimeStart(_) => "runtime.start_failed",
        DownloadError::RuntimeRelease(_) => "runtime.stop_failed",
        DownloadError::SessionMismatch => "session.url_mismatch",
        DownloadError::ArtifactManifest(_) => "artifact.manifest_write_failed",
    }
}
