use crate::provider_client::{
    run_provider_invocation, ProviderInvocation, ProviderInvocationError, ProviderInvocationResult,
    ProviderOperation,
};
use crate::provider_runner::ProviderCommand;

use super::assets::prepare_provider_send_assets;
use super::input::RequestRunInput;

pub(crate) fn provider_send(
    input: &RequestRunInput,
    command: &ProviderCommand,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    let assets = prepare_provider_send_assets(input, command)?;
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Send {
            prompt_file: assets.prompt_file,
            model: input.model.clone(),
            effort: input.effort.clone(),
            files: assets.files,
        },
        env: command.env.clone(),
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}

pub(crate) fn provider_status(
    input: &RequestRunInput,
    command: &ProviderCommand,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Status,
        env: command.env.clone(),
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}

pub(crate) fn provider_capture(
    input: &RequestRunInput,
    command: &ProviderCommand,
    label: &str,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Capture {
            session_id: None,
            label: label.to_string(),
        },
        env: command.env.clone(),
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}

pub(crate) fn provider_capture_session(
    input: &RequestRunInput,
    command: &ProviderCommand,
    session_id: &str,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Capture {
            session_id: Some(session_id.to_string()),
            label: "pre-poll-wait-gate".to_string(),
        },
        env: command.env.clone(),
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}

pub(crate) fn provider_poll(
    input: &RequestRunInput,
    command: &ProviderCommand,
    session_id: &str,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Poll {
            session_id: session_id.to_string(),
            timeout_seconds: input.poll_timeout_seconds,
            artifact_expectation: input.artifact_expectation,
        },
        env: command.env.clone(),
        timeout: input.poll_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}

pub(crate) fn provider_download(
    input: &RequestRunInput,
    command: &ProviderCommand,
    session_id: &str,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    run_provider_invocation(&ProviderInvocation {
        provider_bin: command.provider_bin.clone(),
        args_prefix: command.args_prefix.clone(),
        operation: ProviderOperation::Download {
            session_id: session_id.to_string(),
            artifact_expectation: Some(input.artifact_expectation),
        },
        env: command.env.clone(),
        timeout: input.poll_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    })
}
