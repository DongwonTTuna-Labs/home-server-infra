use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::contracts::ids;
use crate::errors::LifecycleError;

pub(crate) const MAX_DURATION: u64 = 12_000_000;
pub(crate) const MAX_BYTE_CAP: u64 = 16_777_216;
pub(crate) const MAX_POLL_SECONDS: u64 = 10_800;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ParsedProviderMode {
    Fake {
        provider_bin: PathBuf,
    },
    Docker {
        docker_bin: PathBuf,
        artifact_container_root: String,
    },
}

pub(super) fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--kind",
        "--prompt",
        "--fake-runtime",
        "--fake-provider",
        "--docker-slot-provider",
        "--live-send",
        "--require-visual-gate",
        "--provider-bin",
        "--docker-bin",
        "--artifact-container-root",
        "--artifact-expectation",
        "--prompt-file",
        "--file",
        "--request-id",
        "--run-id",
        "--fencing-token",
        "--model",
        "--effort",
        "--ttl-ms",
        "--provider-timeout-ms",
        "--send-timeout-ms",
        "--poll-timeout-seconds",
        "--max-stdout-bytes",
        "--max-stderr-bytes",
        "--runtime-stop-timeout-ms",
        "--runtime-start-timeout-ms",
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

pub(crate) fn required_path(args: &[String], name: &str) -> Result<PathBuf, LifecycleError> {
    required_value(args, name).map(PathBuf::from)
}

pub(crate) fn required_value(args: &[String], name: &str) -> Result<String, LifecycleError> {
    option_value(args, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LifecycleError::Usage(format!("run requires {name}")))
}

pub(crate) fn option_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .filter(|value| !value.starts_with("--"))
        .cloned()
}

pub(crate) fn values_after(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter_map(|(index, arg)| {
            if arg == name {
                args.get(index + 1).filter(|value| !value.starts_with("--"))
            } else {
                None
            }
        })
        .cloned()
        .collect()
}

pub(crate) fn require_flag(
    args: &[String],
    name: &str,
    command: &str,
) -> Result<(), LifecycleError> {
    args.iter()
        .any(|arg| arg == name)
        .then_some(())
        .ok_or_else(|| LifecycleError::Usage(format!("{command} requires {name}")))
}

pub(crate) fn required_command_value(
    args: &[String],
    name: &str,
    command: &str,
) -> Result<String, LifecycleError> {
    option_value(args, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LifecycleError::Usage(format!("{command} requires {name}")))
}

pub(crate) fn validate_request_id(value: &str, name: &str) -> Result<(), LifecycleError> {
    ids::validate_request_id(value)
        .map_err(|_| LifecycleError::Usage(format!("invalid {name}: {value}")))
}

pub(crate) fn validate_run_id(value: &str, name: &str) -> Result<(), LifecycleError> {
    ids::validate_run_id(value)
        .map_err(|_| LifecycleError::Usage(format!("invalid {name}: {value}")))
}

pub(crate) fn validate_session_id(value: &str, name: &str) -> Result<(), LifecycleError> {
    ids::validate_session_id(value)
        .map_err(|_| LifecycleError::Usage(format!("invalid {name}: {value}")))
}

pub(crate) fn validate_slot_id(value: &str, name: &str) -> Result<(), LifecycleError> {
    ids::validate_slot_id(value)
        .map_err(|_| LifecycleError::Usage(format!("invalid {name}: {value}")))
}

pub(crate) fn validate_non_empty_text(value: &str, name: &str) -> Result<(), LifecycleError> {
    ids::validate_non_empty_text(value).map_err(|_| {
        LifecycleError::Usage(format!(
            "invalid {name}: expected 1..4096 UTF-8 bytes without NUL"
        ))
    })
}

pub(crate) fn parse_duration(
    args: &[String],
    name: &str,
    fallback: u64,
) -> Result<u64, LifecycleError> {
    parse_bounded(args, name, fallback, MAX_DURATION)
}

pub(crate) fn parse_byte_cap(
    args: &[String],
    name: &str,
    fallback: usize,
) -> Result<usize, LifecycleError> {
    let value = parse_bounded(args, name, fallback as u64, MAX_BYTE_CAP)?;
    usize::try_from(value).map_err(|_| LifecycleError::Usage(format!("invalid {name}: {value}")))
}

pub(crate) fn parse_poll_seconds(args: &[String], fallback: u64) -> Result<u64, LifecycleError> {
    parse_bounded(args, "--poll-timeout-seconds", fallback, MAX_POLL_SECONDS)
}

pub(crate) fn validate_compatibility_literal(
    args: &[String],
    name: &str,
    expected: u64,
) -> Result<(), LifecycleError> {
    let Some(raw) = option_value(args, name) else {
        return Ok(());
    };
    let actual = parse_decimal(name, &raw)?;
    if actual != expected {
        return Err(LifecycleError::Usage(format!(
            "{name} accepts only the compatibility literal {expected}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_existing_regular_file(
    path: &Path,
    name: &str,
) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        LifecycleError::Usage(format!(
            "{name} must be an existing regular file: {}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LifecycleError::Usage(format!(
            "{name} must be an existing non-symlink regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_existing_executable(path: &Path, name: &str) -> Result<(), LifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        LifecycleError::Usage(format!(
            "{name} must be an existing executable: {}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(LifecycleError::Usage(format!(
            "{name} must be an existing non-symlink executable file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_absolute_container_path(
    path: &Path,
    name: &str,
) -> Result<(), LifecycleError> {
    if !path.is_absolute() || path.as_os_str().is_empty() {
        return Err(LifecycleError::Usage(format!(
            "{name} must be an absolute container path: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_provider_timeout_minimum(
    args: &[String],
    minimum_ms: u64,
    command: &str,
) -> Result<Option<u64>, LifecycleError> {
    let Some(raw) = option_value(args, "--provider-timeout-ms") else {
        return Ok(None);
    };
    let value = parse_decimal("--provider-timeout-ms", &raw)?;
    if !(1..=MAX_DURATION).contains(&value) {
        return Err(LifecycleError::Usage(format!(
            "invalid --provider-timeout-ms: expected 1..={MAX_DURATION}"
        )));
    }
    if value < minimum_ms {
        return Err(LifecycleError::Usage(format!(
            "--provider-timeout-ms for {command} must be at least {minimum_ms}"
        )));
    }
    Ok(Some(value))
}

pub(crate) fn parse_provider_mode(
    args: &[String],
    command: &str,
) -> Result<ParsedProviderMode, LifecycleError> {
    let fake_runtime = args.iter().any(|arg| arg == "--fake-runtime");
    let fake_provider = args.iter().any(|arg| arg == "--fake-provider");
    let provider_bin_present = args.iter().any(|arg| arg == "--provider-bin");
    let docker_mode = args.iter().any(|arg| arg == "--docker-slot-provider");
    let complete_fake = fake_runtime && fake_provider && provider_bin_present;
    if complete_fake == docker_mode {
        return Err(LifecycleError::Usage(format!(
            "{command} requires exactly one provider mode"
        )));
    }
    if fake_runtime || fake_provider || provider_bin_present {
        if !complete_fake {
            return Err(LifecycleError::Usage(format!(
                "{command} fake mode requires the exact bundle --fake-runtime --fake-provider --provider-bin"
            )));
        }
        if args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--docker-bin" | "--artifact-container-root"))
        {
            return Err(LifecycleError::Usage(format!(
                "{command} fake mode forbids Docker options"
            )));
        }
        let provider_bin = PathBuf::from(required_command_value(args, "--provider-bin", command)?);
        validate_existing_executable(&provider_bin, "--provider-bin")?;
        return Ok(ParsedProviderMode::Fake { provider_bin });
    }

    let docker_bin = option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"));
    if option_value(args, "--docker-bin").is_some() {
        validate_existing_executable(&docker_bin, "--docker-bin")?;
    }
    let artifact_container_root = option_value(args, "--artifact-container-root")
        .unwrap_or_else(|| "/broker-artifacts".to_string());
    validate_absolute_container_path(
        Path::new(&artifact_container_root),
        "--artifact-container-root",
    )?;
    Ok(ParsedProviderMode::Docker {
        docker_bin,
        artifact_container_root,
    })
}

fn parse_bounded(
    args: &[String],
    name: &str,
    fallback: u64,
    maximum: u64,
) -> Result<u64, LifecycleError> {
    let Some(raw) = option_value(args, name) else {
        return Ok(fallback);
    };
    let value = parse_decimal(name, &raw)?;
    if !(1..=maximum).contains(&value) {
        return Err(LifecycleError::Usage(format!(
            "invalid {name}: expected 1..={maximum}"
        )));
    }
    Ok(value)
}

fn parse_decimal(name: &str, raw: &str) -> Result<u64, LifecycleError> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(LifecycleError::Usage(format!(
            "invalid {name}: expected a decimal integer"
        )));
    }
    raw.parse::<u64>()
        .map_err(|_| LifecycleError::Usage(format!("invalid {name}: numeric overflow")))
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--kind"
            | "--prompt"
            | "--provider-bin"
            | "--docker-bin"
            | "--artifact-container-root"
            | "--artifact-expectation"
            | "--prompt-file"
            | "--file"
            | "--request-id"
            | "--run-id"
            | "--fencing-token"
            | "--model"
            | "--effort"
            | "--ttl-ms"
            | "--provider-timeout-ms"
            | "--send-timeout-ms"
            | "--poll-timeout-seconds"
            | "--max-stdout-bytes"
            | "--max-stderr-bytes"
            | "--runtime-stop-timeout-ms"
            | "--runtime-start-timeout-ms"
    )
}
