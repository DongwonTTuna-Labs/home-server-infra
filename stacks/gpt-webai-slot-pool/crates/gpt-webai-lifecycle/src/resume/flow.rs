use crate::answer_artifacts::{write_answer_artifacts, AnswerArtifactContext};
use crate::confirmation::confirm_terminal_answer;
use crate::provider_client::{run_provider_invocation, ProviderInvocation, ProviderOperation};
use crate::provider_runner::ProviderCommandContext;
use crate::runtime::RuntimeProbe;
use crate::session_ops::runtime::{
    ensure_session_runtime_started, no_session_runtime_release, no_session_runtime_start,
    stop_owned_session_runtime, SessionRuntimeRelease, SessionRuntimeReleaseInput,
    SessionRuntimeStart, SessionRuntimeStartInput,
};
use crate::sessions::{read_session_record, SessionRecord};

use super::error::{reason_for, ResumeError};
use super::lease::{clear_stale_resume_lease, mark_resume_released};
use super::output::{failed, success, ResumeOutput};
use super::ResumeInput;

struct ResumeFailure {
    error: ResumeError,
    record: Option<SessionRecord>,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
    provider_status: Option<String>,
}

pub(super) fn run(input: ResumeInput, runtime: &dyn RuntimeProbe) -> ResumeOutput {
    match try_resume(&input, runtime) {
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

fn try_resume(
    input: &ResumeInput,
    runtime: &dyn RuntimeProbe,
) -> Result<ResumeOutput, Box<ResumeFailure>> {
    let record = read_session_record(&input.config.state_root, &input.session_id)
        .map_err(|error| failure(ResumeError::from(error), None))?;
    let (request_id, run_id) = record
        .request_binding()
        .map_err(|error| failure(ResumeError::from(error), Some(record.clone())))?;
    clear_stale_resume_lease(input, &record)
        .map_err(|error| failure(error, Some(record.clone())))?;
    let runtime_start = ensure_session_runtime_started(
        SessionRuntimeStartInput {
            config: &input.config,
            slot_id: &record.slot_id,
            mode: &input.runtime_start_mode,
        },
        runtime,
    )
    .map_err(|error| failure(ResumeError::from(error), Some(record.clone())))?;
    let command = input
        .provider_execution
        .command(ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            run_id,
        })
        .map_err(|error| {
            failure_after_runtime_start(input, &record, ResumeError::from(error), &runtime_start)
        })?;
    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin,
        args_prefix: command.args_prefix,
        operation: ProviderOperation::SessionResume {
            session_id: record.session_id.clone(),
        },
        env: command.env,
        timeout: input.provider_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
    .map_err(|error| {
        failure_after_runtime_start(input, &record, ResumeError::from(error), &runtime_start)
    })?;
    if result.summary.session_id.as_deref() != Some(record.session_id.as_str()) {
        return Err(failure_after_runtime_start(
            input,
            &record,
            ResumeError::SessionMismatch,
            &runtime_start,
        ));
    }
    let provider_status = result.summary.status.clone();
    let terminal = if provider_status == "done" {
        Some(confirm_terminal_answer(&result.value).map_err(|error| {
            failure_after_runtime_start(input, &record, ResumeError::from(error), &runtime_start)
        })?)
    } else {
        None
    };
    if let Some(terminal) = &terminal {
        write_answer_artifacts(AnswerArtifactContext {
            config: &input.config,
            path_mode: &command.path_mode,
            request_id,
            run_id,
            terminal,
        })
        .map_err(|error| {
            failure_after_runtime_start(input, &record, ResumeError::from(error), &runtime_start)
        })?;
    }
    let runtime_release = if provider_status == "done" {
        stop_owned_session_runtime(
            SessionRuntimeReleaseInput {
                config: &input.config,
                slot_id: &record.slot_id,
                mode: &input.runtime_release_mode,
            },
            runtime_start.runtime_owned,
        )
        .map_err(|error| {
            failure_with_runtime(
                ResumeError::from(error),
                Some(record.clone()),
                runtime_start.clone(),
                no_session_runtime_release(),
                Some(provider_status.clone()),
            )
        })?
    } else {
        no_session_runtime_release()
    };
    let answer_text_len = terminal.as_ref().map(|terminal| terminal.answer_text_len);
    if provider_status == "done" {
        mark_resume_released(input, &record).map_err(|error| {
            failure_with_runtime(
                ResumeError::SessionWrite(error),
                Some(record.clone()),
                runtime_start.clone(),
                runtime_release.clone(),
                Some(provider_status.clone()),
            )
        })?;
    }
    Ok(success(
        record,
        provider_status,
        answer_text_len,
        runtime_start,
        runtime_release,
    ))
}

fn failure(error: ResumeError, record: Option<SessionRecord>) -> Box<ResumeFailure> {
    failure_with_runtime(
        error,
        record,
        no_session_runtime_start(),
        no_session_runtime_release(),
        None,
    )
}

fn failure_after_runtime_start(
    input: &ResumeInput,
    record: &SessionRecord,
    error: ResumeError,
    runtime_start: &SessionRuntimeStart,
) -> Box<ResumeFailure> {
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
            ResumeError::from(release_error),
            Some(record.clone()),
            runtime_start.clone(),
            no_session_runtime_release(),
            None,
        ),
    }
}

fn failure_with_runtime(
    error: ResumeError,
    record: Option<SessionRecord>,
    runtime_start: SessionRuntimeStart,
    runtime_release: SessionRuntimeRelease,
    provider_status: Option<String>,
) -> Box<ResumeFailure> {
    Box::new(ResumeFailure {
        error,
        record,
        runtime_start,
        runtime_release,
        provider_status,
    })
}
