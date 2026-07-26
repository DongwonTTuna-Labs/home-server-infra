use std::path::PathBuf;
use std::time::Duration;

use super::run::options::{
    parse_byte_cap, parse_duration, parse_poll_seconds, parse_provider_mode, require_flag,
    required_command_value, validate_non_empty_text, validate_provider_timeout_minimum,
    validate_session_id, ParsedProviderMode,
};
use super::{emit_command_outcome, new_operation_id};
use crate::config::SupervisorConfig;
use crate::errors::LifecycleError;
use crate::provider_runner::{
    DockerSlotProviderExecution, HostProviderExecution, ProviderExecution,
};
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::session_ops::provider::ProviderLimits;
use crate::show::{show_session, ShowInput};

pub fn run(args: &[String]) -> Result<u8, LifecycleError> {
    reject_unknown_options(args)?;
    require_flag(args, "--json", "show")?;
    let session_id = required_command_value(args, "--session", "show")?;
    validate_session_id(&session_id, "--session")?;
    let fencing_token = required_command_value(args, "--fencing-token", "show")?;
    validate_non_empty_text(&fencing_token, "--fencing-token")?;
    let provider_mode = parse_provider_mode(args, "show")?;
    let fake_mode = matches!(provider_mode, ParsedProviderMode::Fake { .. });
    let provider_timeout =
        validate_provider_timeout_minimum(args, 200_000, "show")?.unwrap_or(200_000);
    let max_stdout_bytes = parse_byte_cap(args, "--max-stdout-bytes", 1_048_576)?;
    let max_stderr_bytes = parse_byte_cap(args, "--max-stderr-bytes", 262_144)?;
    let _poll_timeout = parse_poll_seconds(args, 300)?;
    let outcome = show_session(ShowInput {
        config: SupervisorConfig::from_env(),
        operation_id: new_operation_id("show")?,
        session_id,
        fencing_token,
        provider_execution: provider_execution(provider_mode),
        runtime_start_mode: runtime_start_mode(args, fake_mode)?,
        runtime_release_mode: runtime_release_mode(args, fake_mode)?,
        provider_limits: ProviderLimits {
            timeout: Duration::from_millis(provider_timeout),
            max_stdout_bytes,
            max_stderr_bytes,
        },
    })
    .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    emit_command_outcome(Ok(outcome))
}

fn runtime_start_mode(
    args: &[String],
    fake_mode: bool,
) -> Result<RuntimeStartMode, LifecycleError> {
    if fake_mode {
        return Ok(RuntimeStartMode::Disabled);
    }
    Ok(RuntimeStartMode::docker(
        docker_bin(args),
        Duration::from_millis(parse_duration(args, "--runtime-start-timeout-ms", 30_000)?),
    ))
}

fn runtime_release_mode(
    args: &[String],
    fake_mode: bool,
) -> Result<RuntimeReleaseMode, LifecycleError> {
    if fake_mode {
        return Ok(RuntimeReleaseMode::LockOnly);
    }
    Ok(RuntimeReleaseMode::docker(
        docker_bin(args),
        Duration::from_millis(parse_duration(args, "--runtime-stop-timeout-ms", 30_000)?),
    ))
}

fn provider_execution(mode: ParsedProviderMode) -> ProviderExecution {
    match mode {
        ParsedProviderMode::Fake { provider_bin } => {
            ProviderExecution::Host(HostProviderExecution {
                provider_bin,
                args_prefix: Vec::new(),
                env: crate::provider_runner::fake_provider_environment(),
            })
        }
        ParsedProviderMode::Docker {
            docker_bin,
            artifact_container_root,
        } => ProviderExecution::DockerSlot(DockerSlotProviderExecution {
            docker_bin,
            artifact_container_root,
        }),
    }
}

fn docker_bin(args: &[String]) -> PathBuf {
    option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"))
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .filter(|value| !value.starts_with("--"))
        .cloned()
}

fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--session",
        "--fencing-token",
        "--fake-runtime",
        "--fake-provider",
        "--provider-bin",
        "--docker-slot-provider",
        "--docker-bin",
        "--artifact-container-root",
        "--provider-timeout-ms",
        "--max-stdout-bytes",
        "--max-stderr-bytes",
        "--poll-timeout-seconds",
        "--runtime-start-timeout-ms",
        "--runtime-stop-timeout-ms",
    ];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--") && !allowed.contains(&arg.as_str()) {
            return Err(LifecycleError::Usage(format!("unknown option: {arg}")));
        }
        index += if matches!(
            arg.as_str(),
            "--session"
                | "--fencing-token"
                | "--provider-bin"
                | "--docker-bin"
                | "--artifact-container-root"
                | "--provider-timeout-ms"
                | "--max-stdout-bytes"
                | "--max-stderr-bytes"
                | "--poll-timeout-seconds"
                | "--runtime-start-timeout-ms"
                | "--runtime-stop-timeout-ms"
        ) {
            2
        } else {
            1
        };
    }
    Ok(())
}
