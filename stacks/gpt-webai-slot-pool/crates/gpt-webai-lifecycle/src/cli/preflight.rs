use std::path::PathBuf;
use std::time::Duration;

use super::run::options::{
    option_value, parse_byte_cap, parse_duration, required_command_value, required_path,
    validate_absolute_container_path, validate_existing_executable, validate_run_id,
    validate_slot_id,
};
use crate::config::SupervisorConfig;
use crate::contracts::cli::CommandOutcome;
use crate::errors::LifecycleError;
use crate::journal::head::{HeadError, HeadStore};
use crate::preflight::{run_preflight, PreflightError, PreflightInput};
use crate::provider_runner::{
    DockerSlotProviderExecution, HostProviderExecution, ProviderExecution,
};
use crate::runtime::{
    docker_runtime_for_provider, DockerStatus, ProviderReadiness, RuntimeObservation,
    StaticRuntimeProbe,
};
use crate::slots;

pub fn run(args: &[String]) -> Result<u8, LifecycleError> {
    if !args.iter().any(|arg| arg == "--json") {
        return Err(LifecycleError::Usage(
            "preflight requires --json".to_string(),
        ));
    }
    reject_unknown_options(args)?;
    let fake_runtime = args.iter().any(|arg| arg == "--fake-runtime");
    let fake_provider = args.iter().any(|arg| arg == "--fake-provider");
    let provider_bin = args.iter().any(|arg| arg == "--provider-bin");
    if !(fake_runtime == fake_provider && fake_provider == provider_bin) {
        return Err(LifecycleError::Usage(
            "preflight fake mode requires the exact bundle --fake-runtime --fake-provider --provider-bin"
                .to_string(),
        ));
    }
    let fake_mode = fake_runtime;
    let docker_mode = args.iter().any(|arg| arg == "--docker-slot-provider");
    if fake_mode == docker_mode {
        return Err(LifecycleError::Usage(
            "preflight requires exactly one provider mode".to_string(),
        ));
    }
    validate_mode_options(args, fake_mode)?;
    let config = SupervisorConfig::from_env();
    let run_id = required_command_value(args, "--run-id", "preflight")?;
    validate_run_id(&run_id, "--run-id")?;
    let slot_id = option_value(args, "--slot");
    if let Some(slot_id) = &slot_id {
        validate_slot_id(slot_id, "--slot")?;
    }
    let provider_timeout = option_value(args, "--provider-timeout-ms")
        .map(|_| parse_duration(args, "--provider-timeout-ms", 1))
        .transpose()?
        .unwrap_or(65_000);
    let input = PreflightInput {
        config: config.clone(),
        provider_execution: provider_execution(args, fake_mode)?,
        slot_id,
        run_id: run_id.clone(),
        provider_timeout: Duration::from_millis(provider_timeout),
        max_stdout_bytes: parse_byte_cap(args, "--max-stdout-bytes", 1_048_576)?,
        max_stderr_bytes: parse_byte_cap(args, "--max-stderr-bytes", 262_144)?,
    };
    let operation_id = super::new_operation_id("preflight")?;
    let guard = match HeadStore::new(&config.state_root).acquire_mutation() {
        Ok(guard) => guard,
        Err(HeadError::LockContended(_)) => {
            let mut outcome = CommandOutcome::select(
                "preflight",
                operation_id,
                "preflight.lock_contended",
                "the lifecycle state-store mutation lock is contended",
                Some("lock.contended".to_string()),
            )
            .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
            outcome.envelope.run_id = Some(run_id);
            outcome.envelope.slot_id = input.slot_id.clone();
            outcome.envelope.cohort = input
                .slot_id
                .as_deref()
                .and_then(crate::allocator::cohort_of)
                .map(str::to_string);
            return super::emit_command_outcome(Ok(outcome));
        }
        Err(error) => {
            let reason = if matches!(error, HeadError::CasConflict) {
                "journal.head_cas_conflict"
            } else {
                "journal.immutable_collision"
            };
            return emit_state_invalid(
                &input,
                &operation_id,
                format!("the lifecycle state-store command guard is invalid: {error}"),
                reason,
            );
        }
    };
    drop(guard);
    if fake_mode {
        let runtime = fake_ready_runtime(&config);
        return emit_preflight_result(
            run_preflight(input.clone(), &runtime, &operation_id),
            &input,
            &operation_id,
        );
    }
    let mut selection_config = config;
    selection_config.status_provider_check = false;
    let runtime = docker_runtime_for_provider(&selection_config, &input.provider_execution);
    emit_preflight_result(
        run_preflight(input.clone(), &runtime, &operation_id),
        &input,
        &operation_id,
    )
}

fn emit_preflight_result(
    result: Result<CommandOutcome, PreflightError>,
    input: &PreflightInput,
    operation_id: &str,
) -> Result<u8, LifecycleError> {
    match result {
        Ok(outcome) => super::emit_command_outcome(Ok(outcome)),
        Err(PreflightError::State(error)) => emit_state_invalid(
            input,
            operation_id,
            format!("the lifecycle preflight local state is invalid: {error}"),
            "journal.immutable_collision",
        ),
        Err(error @ (PreflightError::Outcome(_) | PreflightError::Identifier)) => {
            Err(LifecycleError::Io(std::io::Error::other(error)))
        }
    }
}

fn emit_state_invalid(
    input: &PreflightInput,
    operation_id: &str,
    message: String,
    reason: &str,
) -> Result<u8, LifecycleError> {
    let mut outcome = CommandOutcome::select(
        "preflight",
        operation_id,
        "preflight.state_invalid",
        message,
        Some(reason.to_string()),
    )
    .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    outcome.envelope.run_id = Some(input.run_id.clone());
    outcome.envelope.slot_id = input.slot_id.clone();
    outcome.envelope.cohort = input
        .slot_id
        .as_deref()
        .and_then(crate::allocator::cohort_of)
        .map(str::to_string);
    super::emit_command_outcome(Ok(outcome))
}

fn provider_execution(
    args: &[String],
    fake_mode: bool,
) -> Result<ProviderExecution, LifecycleError> {
    if fake_mode {
        let provider_bin = required_path(args, "--provider-bin")?;
        validate_existing_executable(&provider_bin, "--provider-bin")?;
        return Ok(ProviderExecution::Host(HostProviderExecution {
            provider_bin,
            args_prefix: Vec::new(),
            env: crate::provider_runner::fake_provider_environment(),
        }));
    }
    Ok(ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: option_value(args, "--docker-bin")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker")),
        artifact_container_root: option_value(args, "--artifact-container-root")
            .unwrap_or_else(|| "/broker-artifacts".to_string()),
    }))
}

fn fake_ready_runtime(config: &SupervisorConfig) -> StaticRuntimeProbe {
    StaticRuntimeProbe::new(slots::inventory(config).into_iter().map(|slot| {
        (
            slot.slot_id.0,
            RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        )
    }))
}

fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--fake-runtime",
        "--fake-provider",
        "--docker-slot-provider",
        "--provider-bin",
        "--docker-bin",
        "--artifact-container-root",
        "--slot",
        "--run-id",
        "--provider-timeout-ms",
        "--max-stdout-bytes",
        "--max-stderr-bytes",
    ];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--") && !allowed.contains(&arg.as_str()) {
            return Err(LifecycleError::Usage(format!("unknown option: {arg}")));
        }
        index += if option_takes_value(arg) { 2 } else { 1 };
    }
    Ok(())
}

fn validate_mode_options(args: &[String], fake_mode: bool) -> Result<(), LifecycleError> {
    if fake_mode
        && args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--docker-bin" | "--artifact-container-root"))
    {
        return Err(LifecycleError::Usage(
            "preflight fake mode forbids Docker options".to_string(),
        ));
    }
    if !fake_mode && args.iter().any(|arg| arg == "--provider-bin") {
        return Err(LifecycleError::Usage(
            "preflight Docker mode forbids --provider-bin".to_string(),
        ));
    }
    if let Some(value) = option_value(args, "--docker-bin") {
        validate_existing_executable(PathBuf::from(value).as_path(), "--docker-bin")?;
    }
    if let Some(value) = option_value(args, "--artifact-container-root") {
        validate_absolute_container_path(
            PathBuf::from(value).as_path(),
            "--artifact-container-root",
        )?;
    }
    Ok(())
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--provider-bin"
            | "--docker-bin"
            | "--artifact-container-root"
            | "--slot"
            | "--run-id"
            | "--provider-timeout-ms"
            | "--max-stdout-bytes"
            | "--max-stderr-bytes"
    )
}
