mod args;
mod legacy;
pub(crate) mod options;
mod retry;

use crate::config::SupervisorConfig;
use crate::errors::LifecycleError;
use crate::request::r13::execute_fresh_run;

use super::{acquire_command_guard, emit_command_outcome, new_operation_id};

pub fn run(args: &[String]) -> Result<u8, LifecycleError> {
    let config = SupervisorConfig::from_env();
    let command = args::parse_run_command(args, config)?;
    let _ = command.fake_mode;
    let operation_id = new_operation_id("run")?;
    drop(crate::provider_runner::ensure_private_state_root(
        &command.input.config.state_root,
    )?);
    let guard = match acquire_command_guard(&command.input.config.state_root, "run", &operation_id)?
    {
        Ok(guard) => guard,
        Err(exit_code) => return Ok(exit_code),
    };
    if let Some(prompt) = command.legacy_prompt.as_deref() {
        legacy::materialize_prompt_file(
            &command.input.config.state_root,
            &command.input.prompt_file,
            prompt,
        )?;
    }
    let outcome = execute_fresh_run(command.input, operation_id, guard)
        .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    emit_command_outcome(Ok(outcome))
}
