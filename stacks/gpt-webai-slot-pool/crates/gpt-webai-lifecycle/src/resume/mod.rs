use std::time::Duration;

mod error;
mod flow;
mod lease;
mod output;

use crate::config::SupervisorConfig;
use crate::contracts::cli::CommandOutcome;
use crate::provider_runner::ProviderExecution;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::runtime::{docker_runtime_for_provider, RuntimeProbe};
use crate::session_ops::executor::{execute_resume, SessionExecutorError, SessionExecutorInput};
pub use crate::session_ops::provider::ProviderLimits;

pub use output::ResumeOutput;

#[derive(Clone, Debug)]
pub struct ResumeInput {
    pub config: SupervisorConfig,
    pub session_id: String,
    pub fencing_token: String,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub provider_timeout: Duration,
    pub poll_timeout_seconds: u64,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

pub struct ResumeR13Input {
    pub config: SupervisorConfig,
    pub operation_id: String,
    pub session_id: String,
    pub fencing_token: String,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub provider_limits: ProviderLimits,
    pub poll_timeout_seconds: u64,
}

pub fn resume_session_r13(input: ResumeR13Input) -> Result<CommandOutcome, SessionExecutorError> {
    let poll_timeout_seconds = input.poll_timeout_seconds;
    execute_resume(
        SessionExecutorInput {
            config: input.config,
            operation_id: input.operation_id,
            session_id: input.session_id,
            fencing_token: input.fencing_token,
            provider_execution: input.provider_execution,
            runtime_start_mode: input.runtime_start_mode,
            runtime_release_mode: input.runtime_release_mode,
            provider_limits: input.provider_limits,
        },
        poll_timeout_seconds,
    )
}

pub fn resume_session(input: ResumeInput) -> ResumeOutput {
    let runtime = docker_runtime_for_provider(&input.config, &input.provider_execution);
    resume_session_with_runtime(input, &runtime)
}

pub fn resume_session_with_runtime(input: ResumeInput, runtime: &dyn RuntimeProbe) -> ResumeOutput {
    flow::run(input, runtime)
}
