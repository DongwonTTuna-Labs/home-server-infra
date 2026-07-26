use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::SupervisorConfig;
use crate::errors::LifecycleError;
use crate::provider_runner::{DockerSlotProviderExecution, ProviderExecution};
use crate::request::artifact_expectation::ArtifactExpectation;
use crate::request::run::RequestRunInput;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};

use super::args::RunCommand;
use super::options::{
    option_value, parse_byte_cap, parse_duration, parse_poll_seconds,
    validate_absolute_container_path, validate_compatibility_literal, validate_existing_executable,
    validate_existing_regular_file, validate_non_empty_text, validate_provider_timeout_minimum,
    validate_request_id, validate_run_id, values_after,
};
use super::retry;

pub(super) fn is_legacy_surface(args: &[String]) -> bool {
    !args.iter().any(|arg| arg == "--json")
}

pub(super) fn parse(
    args: &[String],
    config: SupervisorConfig,
) -> Result<RunCommand, LifecycleError> {
    reject_unknown_options(args)?;
    let kind = option_value(args, "--kind")
        .ok_or_else(|| LifecycleError::Usage("run requires --kind pro|xhigh".to_string()))?;
    let (model, effort) = model_effort(&kind)?;
    let prompt = option_value(args, "--prompt")
        .ok_or_else(|| LifecycleError::Usage("run requires --prompt PROMPT".to_string()))?;
    validate_non_empty_text(&prompt, "--prompt")?;
    let nonce = nonce();
    let request_id = option_value(args, "--request-id").unwrap_or_else(|| format!("req-{nonce}"));
    let run_id = option_value(args, "--run-id").unwrap_or_else(|| format!("run-{nonce}"));
    let fencing_token =
        option_value(args, "--fencing-token").unwrap_or_else(|| format!("fence-{nonce}"));
    validate_request_id(&request_id, "--request-id")?;
    validate_run_id(&run_id, "--run-id")?;
    validate_non_empty_text(&fencing_token, "--fencing-token")?;
    validate_compatibility_literal(args, "--ttl-ms", 300_000)?;
    validate_compatibility_literal(args, "--send-timeout-ms", 30_000)?;
    validate_optional_paths(args)?;
    let prompt_file = prompt_file_path(&config.state_root, &run_id);
    let poll_timeout_seconds = legacy_poll_timeout_seconds(args, &kind)?;
    let files = values_after(args, "--file")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for file in &files {
        validate_existing_regular_file(file, "--file")?;
    }
    let poll_deadline = 200_000 + poll_timeout_seconds * 1_000;
    let provider_timeout_override =
        validate_provider_timeout_minimum(args, poll_deadline.max(320_000), "run")?;

    let input = RequestRunInput {
        config,
        provider_execution: ProviderExecution::DockerSlot(DockerSlotProviderExecution {
            docker_bin: docker_bin(args),
            artifact_container_root: option_value(args, "--artifact-container-root")
                .unwrap_or_else(|| "/broker-artifacts".to_string()),
        }),
        runtime_start_mode: RuntimeStartMode::docker(
            docker_bin(args),
            Duration::from_millis(parse_duration(args, "--runtime-start-timeout-ms", 30_000)?),
        ),
        runtime_release_mode: RuntimeReleaseMode::docker(
            docker_bin(args),
            Duration::from_millis(parse_duration(args, "--runtime-stop-timeout-ms", 30_000)?),
        ),
        pre_send_visual_gate: true,
        pre_poll_wait_gate: true,
        download_artifacts_after_poll: true,
        artifact_expectation: legacy_artifact_expectation(args, &prompt)?,
        prompt_file,
        files,
        request_id,
        run_id,
        fencing_token,
        model,
        effort,
        ttl_ms: 300_000,
        send_retry_delays: retry::send_retry_delays(false),
        provider_limit_retry_delays: retry::provider_limit_retry_delays(false),
        send_process_timeout: Duration::from_millis(provider_timeout_override.unwrap_or(65_000)),
        poll_timeout_seconds,
        poll_process_timeout: Duration::from_millis(
            provider_timeout_override
                .unwrap_or_else(|| 170_000 + poll_timeout_seconds * 1_000 + 30_000),
        ),
        max_stdout_bytes: parse_byte_cap(args, "--max-stdout-bytes", 1_048_576)?,
        max_stderr_bytes: parse_byte_cap(args, "--max-stderr-bytes", 262_144)?,
    };
    Ok(RunCommand {
        input,
        fake_mode: false,
        legacy_prompt: Some(prompt),
    })
}

fn legacy_artifact_expectation(
    args: &[String],
    prompt: &str,
) -> Result<ArtifactExpectation, LifecycleError> {
    match option_value(args, "--artifact-expectation") {
        Some(value) => ArtifactExpectation::parse(&value).ok_or_else(|| {
            LifecycleError::Usage(format!(
                "unsupported --artifact-expectation: {value}; expected none, optional, required, or claimed"
            ))
        }),
        None => Ok(ArtifactExpectation::from_prompt(prompt)),
    }
}

fn model_effort(kind: &str) -> Result<(String, String), LifecycleError> {
    match kind {
        "pro" => Ok(("pro".to_string(), "standard".to_string())),
        "xhigh" => Ok(("xhigh".to_string(), "high".to_string())),
        other => Err(LifecycleError::Usage(format!(
            "unsupported run kind: {other}"
        ))),
    }
}

fn legacy_poll_timeout_seconds(args: &[String], kind: &str) -> Result<u64, LifecycleError> {
    if option_value(args, "--poll-timeout-seconds").is_some() {
        return parse_poll_seconds(args, 300);
    }
    Ok(timeout_env(kind).unwrap_or(300).clamp(1, 10_800))
}

fn timeout_env(kind: &str) -> Option<u64> {
    let name = match kind {
        "pro" | "gptpro" => "GPTPRO_TIMEOUT",
        "xhigh" | "gptxhigh" => "GPTXHIGH_TIMEOUT",
        _ => return None,
    };
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn prompt_file_path(state_root: &Path, run_id: &str) -> PathBuf {
    state_root
        .join("requests")
        .join("legacy-inputs")
        .join(safe_key(run_id))
        .join("prompt.md")
}

pub(super) fn materialize_prompt_file(
    state_root: &Path,
    prompt_file: &Path,
    prompt: &str,
) -> Result<(), LifecycleError> {
    let prompt_dir = prompt_file.parent().ok_or_else(|| {
        LifecycleError::Io(std::io::Error::other("legacy prompt parent is missing"))
    })?;
    crate::provider_runner::create_private_directory(state_root, prompt_dir)?;
    let bytes = prompt.as_bytes();
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(prompt_file)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            File::open(prompt_dir)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(prompt_file)? != bytes {
                return Err(LifecycleError::Io(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "legacy prompt collision",
                )));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn docker_bin(args: &[String]) -> PathBuf {
    option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"))
}

fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--kind",
        "--prompt",
        "--file",
        "--docker-bin",
        "--artifact-container-root",
        "--artifact-expectation",
        "--request-id",
        "--run-id",
        "--fencing-token",
        "--ttl-ms",
        "--send-timeout-ms",
        "--provider-timeout-ms",
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

fn validate_optional_paths(args: &[String]) -> Result<(), LifecycleError> {
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
        "--kind"
            | "--prompt"
            | "--file"
            | "--docker-bin"
            | "--artifact-container-root"
            | "--artifact-expectation"
            | "--request-id"
            | "--run-id"
            | "--fencing-token"
            | "--ttl-ms"
            | "--send-timeout-ms"
            | "--provider-timeout-ms"
            | "--poll-timeout-seconds"
            | "--max-stdout-bytes"
            | "--max-stderr-bytes"
            | "--runtime-stop-timeout-ms"
            | "--runtime-start-timeout-ms"
    )
}

fn nonce() -> String {
    let nanos = u128::from(crate::config::now_ms()) * 1_000_000;
    format!("{}-{nanos}", std::process::id())
}

fn safe_key(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = safe.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}
