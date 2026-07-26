use crate::cleanup::{cleanup_state, CleanupMode};
use crate::config::SupervisorConfig;
use crate::contracts::cli::{CommandOutcome, CommandOutcomeError};
use crate::errors::LifecycleError;
use crate::journal::head::{HeadError, HeadStore, MutationGuard};
use crate::json_contract;
use crate::runtime::DockerRuntime;
use crate::status;
use std::ffi::OsString;

pub mod allocate;
pub mod download;
pub mod preflight;
pub mod release;
pub mod resume;
pub mod run;
pub mod show;
pub mod state_rebuild;

const VALUE_OPTIONS: &[&str] = &[
    "--artifact-container-root",
    "--artifact-expectation",
    "--docker-bin",
    "--effort",
    "--fencing-token",
    "--file",
    "--kind",
    "--max-stderr-bytes",
    "--max-stdout-bytes",
    "--model",
    "--poll-timeout-seconds",
    "--prompt",
    "--prompt-file",
    "--provider-bin",
    "--provider-timeout-ms",
    "--request-id",
    "--run-id",
    "--runtime-start-timeout-ms",
    "--runtime-stop-timeout-ms",
    "--send-timeout-ms",
    "--session",
    "--slot",
    "--ttl-ms",
];

pub fn run_os(args: Vec<OsString>) -> Result<u8, LifecycleError> {
    let args = args
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| LifecycleError::Usage("arguments must be valid UTF-8".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    run(args)
}

pub fn run(args: Vec<String>) -> Result<u8, LifecycleError> {
    crate::failpoint::validate_requested().map_err(LifecycleError::Usage)?;
    validate_lexical_args(&args)?;
    let Some(command) = args.first().map(String::as_str) else {
        return Err(LifecycleError::Usage(usage()));
    };
    match command {
        "status" => run_status(&args[1..]),
        "allocate" => allocate::run(&args[1..]),
        "preflight" => preflight::run(&args[1..]),
        "run" => run::run(&args[1..]),
        "show" => show::run(&args[1..]),
        "resume" => resume::run(&args[1..]),
        "download" => download::run(&args[1..]),
        "release" => release::run(&args[1..]),
        "cleanup" => run_cleanup(&args[1..]),
        "state-rebuild" => state_rebuild::run(&args[1..]),
        "constants" => run_constants(&args[1..]).map(|()| 0),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(0)
        }
        other => Err(LifecycleError::Usage(format!(
            "unknown command: {other}\n{}",
            usage()
        ))),
    }
}

fn validate_lexical_args(args: &[String]) -> Result<(), LifecycleError> {
    let Some((_, command_args)) = args.split_first() else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut file_count = 0_usize;
    let mut index = 0_usize;
    while index < command_args.len() {
        let token = &command_args[index];
        if token == "--" {
            return Err(LifecycleError::Usage(
                "the Rust lifecycle CLI does not accept --".to_string(),
            ));
        }
        if token.starts_with("--") && token.contains('=') {
            return Err(LifecycleError::Usage(format!(
                "Rust lifecycle options require separate tokens: {token}"
            )));
        }
        if !token.starts_with("--") {
            return Err(LifecycleError::Usage(format!(
                "unexpected positional argument: {token}"
            )));
        }
        if token == "--file" {
            file_count += 1;
            if file_count > 64 {
                return Err(LifecycleError::Usage(
                    "no more than 64 --file options are allowed".to_string(),
                ));
            }
        } else if !seen.insert(token.as_str()) {
            return Err(LifecycleError::Usage(format!(
                "duplicate singleton option: {token}"
            )));
        }
        if VALUE_OPTIONS.contains(&token.as_str()) {
            let Some(value) = command_args.get(index + 1) else {
                return Err(LifecycleError::Usage(format!("missing value for {token}")));
            };
            if value.is_empty() || value.starts_with("--") {
                return Err(LifecycleError::Usage(format!("missing value for {token}")));
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn run_status(args: &[String]) -> Result<u8, LifecycleError> {
    let json = args.iter().any(|arg| arg == "--json");
    let legacy_kv = args.iter().any(|arg| arg == "--legacy-kv");
    if args
        .iter()
        .any(|arg| arg != "--json" && arg != "--legacy-kv")
    {
        return Err(LifecycleError::Usage(
            "status accepts only --json or --legacy-kv".to_string(),
        ));
    }
    if json && legacy_kv {
        return Err(LifecycleError::Usage(
            "status accepts at most one of --json or --legacy-kv".to_string(),
        ));
    }

    let config = SupervisorConfig::from_env();
    let runtime = DockerRuntime::new(&config);
    if legacy_kv || !json {
        let status = status::build_status(&config, &runtime)?;
        status::write_legacy_kv(&status, std::io::stdout())?;
        return Ok(0);
    }

    let operation_id = new_operation_id("status")?;
    if let Err(error) = load_status_projection(&config) {
        return emit_command_outcome(CommandOutcome::select(
            "status",
            operation_id,
            "status.state_invalid",
            error,
            Some("journal.immutable_collision".to_string()),
        ));
    }
    let guard = match acquire_command_guard(&config.state_root, "status", &operation_id)? {
        Ok(guard) => guard,
        Err(exit_code) => return Ok(exit_code),
    };
    let projection = match load_status_projection(&config) {
        Ok(projection) => projection,
        Err(error) => {
            drop(guard);
            return emit_command_outcome(CommandOutcome::select(
                "status",
                operation_id,
                "status.state_invalid",
                error,
                Some("journal.immutable_collision".to_string()),
            ));
        }
    };
    drop(guard);
    let decision = status::aggregate_r13_status(&config, &runtime, &projection);
    let mut outcome = CommandOutcome::select(
        "status",
        operation_id,
        decision.result_kind,
        decision.message,
        decision.reason,
    )
    .map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    outcome.envelope.slot_id = decision.slot_id;
    emit_command_outcome(Ok(outcome))
}

fn load_status_projection(
    config: &SupervisorConfig,
) -> Result<crate::contracts::projection::ProjectionState, String> {
    let seeds = crate::session_ops::journal::persisted_session_seeds(config)
        .map_err(|error| format!("the persisted-session bootstrap state is invalid: {error}"))?;
    crate::journal::EventStore::new(&config.state_root)
        .replay(&seeds)
        .map(|projection| projection.state)
        .map_err(|error| format!("the lifecycle state store cannot be replayed: {error}"))
}

fn run_constants(args: &[String]) -> Result<(), LifecycleError> {
    if !args.is_empty() {
        return Err(LifecycleError::Usage(
            "constants takes no arguments".to_string(),
        ));
    }
    println!("EX_OK=0");
    println!("EX_USAGE=2");
    println!("EX_HARD=70");
    println!("EX_LOCK=75");
    Ok(())
}

fn run_cleanup(args: &[String]) -> Result<u8, LifecycleError> {
    let mode = match args {
        [json, mode] if json == "--json" && mode == "--dry-run" => CleanupMode::DryRun,
        [json, mode] if json == "--json" && mode == "--apply" => CleanupMode::Apply,
        _ => {
            return Err(LifecycleError::Usage(
                "cleanup requires --json followed by exactly one mode: --dry-run or --apply"
                    .to_string(),
            ));
        }
    };
    let config = SupervisorConfig::from_env();
    let operation_id = new_operation_id("cleanup")?;
    let _guard = match acquire_command_guard(&config.state_root, "cleanup", &operation_id)? {
        Ok(guard) => guard,
        Err(exit_code) => return Ok(exit_code),
    };
    let inspected = cleanup_state(&config.state_root, mode);
    let result_kind = match mode {
        CleanupMode::DryRun => "cleanup.plan",
        CleanupMode::Apply if inspected.skipped == 0 => "cleanup.applied",
        CleanupMode::Apply => "cleanup.partial_failure",
    };
    let reason = (inspected.skipped > 0).then(|| result_kind.to_string());
    emit_command_outcome(CommandOutcome::select(
        "cleanup",
        operation_id,
        result_kind,
        format!(
            "cleanup mode={} stale_holders={} stale_locks={} removed_holders={} removed_locks={} skipped={}",
            inspected.mode,
            inspected.stale_holders,
            inspected.stale_locks,
            inspected.removed_holders,
            inspected.removed_locks,
            inspected.skipped
        ),
        reason,
    ))
}

pub(crate) fn emit_command_outcome(
    outcome: Result<CommandOutcome, CommandOutcomeError>,
) -> Result<u8, LifecycleError> {
    let outcome = outcome.map_err(|error| LifecycleError::Io(std::io::Error::other(error)))?;
    json_contract::print_json(&outcome.envelope)?;
    Ok(outcome.exit_code)
}

pub(crate) fn acquire_command_guard(
    state_root: &std::path::Path,
    command: &str,
    operation_id: &str,
) -> Result<Result<MutationGuard, u8>, LifecycleError> {
    match HeadStore::new(state_root).acquire_mutation() {
        Ok(guard) => Ok(Ok(guard)),
        Err(HeadError::LockContended(_)) => emit_command_outcome(CommandOutcome::select(
            command,
            operation_id,
            format!("{}.lock_contended", command.replace('-', "_")),
            "the lifecycle state-store mutation lock is contended",
            Some("lock.contended".to_string()),
        ))
        .map(Err),
        Err(error) if command == "status" => {
            let reason = if matches!(error, HeadError::CasConflict) {
                "journal.head_cas_conflict"
            } else {
                "journal.immutable_collision"
            };
            emit_command_outcome(CommandOutcome::select(
                command,
                operation_id,
                "status.state_invalid",
                format!("the lifecycle state-store command guard is invalid: {error}"),
                Some(reason.to_string()),
            ))
            .map(Err)
        }
        Err(error) if matches!(command, "cleanup" | "allocate") => {
            let result_kind = format!("{command}.state_invalid");
            emit_command_outcome(CommandOutcome::select(
                command,
                operation_id,
                &result_kind,
                format!("the lifecycle state-store command guard is invalid: {error}"),
                Some(result_kind.clone()),
            ))
            .map(Err)
        }
        Err(error) => Err(LifecycleError::Io(std::io::Error::other(error))),
    }
}

pub(crate) fn new_operation_id(command: &str) -> Result<String, LifecycleError> {
    Ok(format!(
        "{command}-{}-{}",
        std::process::id(),
        crate::config::now_ms()
    ))
}

fn usage() -> String {
    "usage: gpt-webai-lifecycle <status [--json|--legacy-kv]|cleanup --json (--dry-run|--apply)|state-rebuild --json --check-only|allocate --json [--dry-run]|preflight --json --docker-slot-provider --run-id RUN|run --kind pro|xhigh [--file PATH ...] --prompt PROMPT|show [--kind pro|xhigh] --session SESSION|resume [--kind pro|xhigh] --session SESSION|download [--kind pro|xhigh] --session SESSION|release (--slot SLOT|--session SESSION) [--fencing-token TOKEN] [--stop-runtime]|constants>"
        .to_string()
}
