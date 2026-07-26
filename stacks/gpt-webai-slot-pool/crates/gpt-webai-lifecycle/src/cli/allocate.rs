mod output;

use std::path::PathBuf;

use crate::cli::run::options::{
    option_value, require_flag, required_command_value, validate_compatibility_literal,
    validate_existing_executable, validate_non_empty_text, validate_request_id, validate_run_id,
};
use crate::config::SupervisorConfig;
use crate::contracts::cli::CommandOutcome;
use crate::errors::LifecycleError;
use crate::records;
use crate::runtime::DockerRuntime;
use crate::slots::{select_fresh_slot_with_rotation, AllocationCandidate};
use crate::status;

use output::with_allocation_identity;

use super::{acquire_command_guard, emit_command_outcome, new_operation_id};

pub fn run(args: &[String]) -> Result<u8, LifecycleError> {
    reject_unknown_options(args)?;
    require_flag(args, "--json", "allocate")?;
    if args.iter().any(|arg| arg == "--runtime-start-timeout-ms") {
        return Err(LifecycleError::Usage(
            "allocate rejects --runtime-start-timeout-ms in read-only R13".to_string(),
        ));
    }

    let request_id = required_command_value(args, "--request-id", "allocate")?;
    let run_id = required_command_value(args, "--run-id", "allocate")?;
    let fencing_token = required_command_value(args, "--fencing-token", "allocate")?;
    validate_request_id(&request_id, "--request-id")?;
    validate_run_id(&run_id, "--run-id")?;
    validate_non_empty_text(&fencing_token, "--fencing-token")?;
    validate_compatibility_literal(args, "--ttl-ms", 300_000)?;

    let docker_bin = option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"));
    if option_value(args, "--docker-bin").is_some() {
        validate_existing_executable(&docker_bin, "--docker-bin")?;
    }

    let operation_id = new_operation_id("allocate")?;
    let config = SupervisorConfig::from_env();
    let runtime = DockerRuntime::with_docker_bin(&config, docker_bin);
    let status = match status::build_status(&config, &runtime) {
        Ok(status) => status,
        Err(error) => {
            let outcome = CommandOutcome::select(
                "allocate",
                operation_id,
                "allocate.state_invalid",
                error.to_string(),
                Some("allocate.state_invalid".to_string()),
            )
            .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
            let outcome = with_allocation_identity(outcome, request_id, run_id, None, None);
            return emit_command_outcome(Ok(outcome));
        }
    };
    let _guard = match acquire_command_guard(&config.state_root, "allocate", &operation_id)? {
        Ok(guard) => guard,
        Err(exit_code) => return Ok(exit_code),
    };
    let candidates = status
        .slots
        .iter()
        .map(|slot| AllocationCandidate {
            slot_id: crate::slots::SlotId(slot.slot_id.clone()),
            account_group: crate::slots::AccountGroupId(slot.account_group.clone()),
            allocatable: slot.allocatable,
        })
        .collect::<Vec<_>>();
    let cursor = records::read_group_cursor(&config.state_root);
    let slot_rotation = records::read_slot_rotation_cursors(
        &config.state_root,
        candidates
            .iter()
            .map(|candidate| candidate.account_group.0.clone()),
    );
    let (cursor, slot_rotation) = match (cursor, slot_rotation) {
        (Ok(cursor), Ok(slot_rotation)) => (cursor, slot_rotation),
        (cursor, slot_rotation) => {
            let message = cursor
                .err()
                .map(|error| error.to_string())
                .or_else(|| slot_rotation.err().map(|error| error.to_string()))
                .unwrap_or_else(|| "allocator state invalid".to_string());
            let outcome = CommandOutcome::select(
                "allocate",
                operation_id,
                "allocate.state_invalid",
                message,
                Some("allocate.state_invalid".to_string()),
            )
            .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
            let outcome = with_allocation_identity(outcome, request_id, run_id, None, None);
            return emit_command_outcome(Ok(outcome));
        }
    };
    let decision = select_fresh_slot_with_rotation(
        &candidates,
        cursor
            .as_ref()
            .map(|record| record.last_preferred_group.as_str()),
        &slot_rotation,
    );

    let Some(decision) = decision else {
        let outcome = CommandOutcome::select(
            "allocate",
            operation_id,
            "allocate.pool_busy",
            "read-only allocation preview found no allocatable slot",
            None,
        )
        .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
        let outcome = with_allocation_identity(outcome, request_id, run_id, None, None);
        return emit_command_outcome(Ok(outcome));
    };

    let outcome = CommandOutcome::select(
        "allocate",
        operation_id,
        "allocate.dry_run_candidate",
        "read-only allocation preview; no cursor, lock, journal, or runtime mutation",
        None,
    )
    .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    let outcome = with_allocation_identity(
        outcome,
        request_id,
        run_id,
        Some(decision.slot_id.0),
        Some(decision.allocated_group.0),
    );
    emit_command_outcome(Ok(outcome))
}

fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--dry-run",
        "--request-id",
        "--run-id",
        "--fencing-token",
        "--ttl-ms",
        "--docker-bin",
        "--runtime-start-timeout-ms",
    ];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--") && !allowed.contains(&arg.as_str()) {
            return Err(LifecycleError::Usage(format!("unknown option: {arg}")));
        }
        index += if matches!(
            arg.as_str(),
            "--request-id"
                | "--run-id"
                | "--fencing-token"
                | "--ttl-ms"
                | "--docker-bin"
                | "--runtime-start-timeout-ms"
        ) {
            2
        } else {
            1
        };
    }
    Ok(())
}
