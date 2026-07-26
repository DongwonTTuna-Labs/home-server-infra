use std::path::PathBuf;
use std::time::Duration;

use crate::cli::run::options::{
    option_value, parse_duration, require_flag, validate_existing_executable,
    validate_non_empty_text, validate_session_id, validate_slot_id,
};
use crate::errors::LifecycleError;

pub(super) struct ReleaseArgs {
    pub fencing_token: Option<String>,
    pub slot_id: Option<String>,
    pub session_id: Option<String>,
    pub docker_bin: PathBuf,
    pub runtime_stop_timeout: Duration,
}

pub(super) fn parse(args: &[String]) -> Result<ReleaseArgs, LifecycleError> {
    reject_unknown_options(args)?;
    require_flag(args, "--json", "release")?;
    let fencing_token = option_value(args, "--fencing-token");
    if let Some(token) = &fencing_token {
        validate_non_empty_text(token, "--fencing-token")?;
    }
    let slot_id = option_value(args, "--slot");
    let session_id = option_value(args, "--session");
    if slot_id.is_some() == session_id.is_some() {
        return Err(LifecycleError::Usage(
            "release requires exactly one of --slot or --session".to_string(),
        ));
    }
    if let Some(slot_id) = &slot_id {
        validate_slot_id(slot_id, "--slot")?;
    }
    if let Some(session_id) = &session_id {
        validate_session_id(session_id, "--session")?;
    }
    let docker_bin = option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"));
    if option_value(args, "--docker-bin").is_some() {
        validate_existing_executable(&docker_bin, "--docker-bin")?;
    }
    Ok(ReleaseArgs {
        fencing_token,
        slot_id,
        session_id,
        docker_bin,
        runtime_stop_timeout: Duration::from_millis(parse_duration(
            args,
            "--runtime-stop-timeout-ms",
            30_000,
        )?),
    })
}

fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--slot",
        "--session",
        "--fencing-token",
        "--stop-runtime",
        "--docker-bin",
        "--runtime-stop-timeout-ms",
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

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--slot" | "--session" | "--fencing-token" | "--docker-bin" | "--runtime-stop-timeout-ms"
    )
}
