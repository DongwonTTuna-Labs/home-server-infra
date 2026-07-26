mod args;

use crate::config::SupervisorConfig;
use crate::errors::LifecycleError;
use crate::session_ops::release::{execute_explicit_release, ExplicitReleaseInput};

use super::{emit_command_outcome, new_operation_id};

pub fn run(raw_args: &[String]) -> Result<u8, LifecycleError> {
    let args = args::parse(raw_args)?;
    let outcome = execute_explicit_release(ExplicitReleaseInput {
        config: SupervisorConfig::from_env(),
        operation_id: new_operation_id("release")?,
        session_id: args.session_id,
        slot_id: args.slot_id,
        fencing_token: args.fencing_token,
        docker_bin: args.docker_bin,
        runtime_stop_timeout: args.runtime_stop_timeout,
    })
    .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    emit_command_outcome(Ok(outcome))
}
