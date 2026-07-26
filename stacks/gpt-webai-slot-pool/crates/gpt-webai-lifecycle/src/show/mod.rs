use crate::config::SupervisorConfig;
use crate::contracts::cli::CommandOutcome;
use crate::provider_runner::ProviderExecution;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::session_ops::executor::{execute_show, SessionExecutorError, SessionExecutorInput};
pub use crate::session_ops::provider::ProviderLimits;

pub struct ShowInput {
    pub config: SupervisorConfig,
    pub operation_id: String,
    pub session_id: String,
    pub fencing_token: String,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub provider_limits: ProviderLimits,
}

pub fn show_session(input: ShowInput) -> Result<CommandOutcome, SessionExecutorError> {
    execute_show(SessionExecutorInput {
        config: input.config,
        operation_id: input.operation_id,
        session_id: input.session_id,
        fencing_token: input.fencing_token,
        provider_execution: input.provider_execution,
        runtime_start_mode: input.runtime_start_mode,
        runtime_release_mode: input.runtime_release_mode,
        provider_limits: input.provider_limits,
    })
}
