use std::path::PathBuf;
use std::time::Duration;

use crate::config::SupervisorConfig;
use crate::errors::LifecycleError;
use crate::provider_runner::{
    DockerSlotProviderExecution, HostProviderExecution, ProviderExecution,
};
use crate::request::artifact_expectation::ArtifactExpectation;
use crate::request::run::RequestRunInput;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};

use super::options::{
    option_value, parse_byte_cap, parse_duration, parse_poll_seconds, reject_unknown_options,
    required_path, required_value, validate_absolute_container_path,
    validate_compatibility_literal, validate_existing_executable, validate_existing_regular_file,
    validate_non_empty_text, validate_provider_timeout_minimum, validate_request_id,
    validate_run_id, values_after,
};
use super::retry;

pub(super) struct RunCommand {
    pub input: RequestRunInput,
    pub fake_mode: bool,
    pub legacy_prompt: Option<String>,
}

pub(super) fn parse_run_command(
    args: &[String],
    config: SupervisorConfig,
) -> Result<RunCommand, LifecycleError> {
    if super::legacy::is_legacy_surface(args) {
        return super::legacy::parse(args, config);
    }
    require_json(args)?;
    reject_unknown_options(args)?;
    let fake_mode = provider_mode(args)?;
    let request_id = required_value(args, "--request-id")?;
    let run_id = required_value(args, "--run-id")?;
    let fencing_token = required_value(args, "--fencing-token")?;
    validate_request_id(&request_id, "--request-id")?;
    validate_run_id(&run_id, "--run-id")?;
    validate_non_empty_text(&fencing_token, "--fencing-token")?;
    let (model, effort) = model_effort(args)?;
    let prompt_file = required_path(args, "--prompt-file")?;
    validate_existing_regular_file(&prompt_file, "--prompt-file")?;
    let files = values_after(args, "--file")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for file in &files {
        validate_existing_regular_file(file, "--file")?;
    }
    validate_compatibility_literal(args, "--ttl-ms", 300_000)?;
    validate_compatibility_literal(args, "--send-timeout-ms", 30_000)?;
    let poll_timeout_seconds = parse_poll_seconds(args, 300)?;
    let poll_deadline = 200_000 + poll_timeout_seconds * 1_000;
    let provider_timeout_override =
        validate_provider_timeout_minimum(args, poll_deadline.max(320_000), "run")?;
    let input = RequestRunInput {
        config,
        provider_execution: provider_execution(args, fake_mode)?,
        runtime_start_mode: runtime_start_mode(args, fake_mode)?,
        runtime_release_mode: runtime_release_mode(args, fake_mode)?,
        pre_send_visual_gate: !fake_mode,
        pre_poll_wait_gate: !fake_mode,
        download_artifacts_after_poll: !fake_mode,
        artifact_expectation: artifact_expectation(args)?,
        prompt_file,
        files,
        request_id,
        run_id,
        fencing_token,
        model,
        effort,
        ttl_ms: 300_000,
        send_retry_delays: retry::send_retry_delays(fake_mode),
        provider_limit_retry_delays: retry::provider_limit_retry_delays(fake_mode),
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
        fake_mode,
        legacy_prompt: None,
    })
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
    require_live_docker_gate(args)?;
    validate_docker_options(args)?;
    Ok(ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: docker_bin(args),
        artifact_container_root: option_value(args, "--artifact-container-root")
            .unwrap_or_else(|| "/broker-artifacts".to_string()),
    }))
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

fn provider_mode(args: &[String]) -> Result<bool, LifecycleError> {
    let fake_runtime = args.iter().any(|arg| arg == "--fake-runtime");
    let fake_provider = args.iter().any(|arg| arg == "--fake-provider");
    let provider_bin = args.iter().any(|arg| arg == "--provider-bin");
    if !(fake_runtime == fake_provider && fake_provider == provider_bin) {
        return Err(LifecycleError::Usage(
            "fake run requires the exact bundle --fake-runtime --fake-provider --provider-bin"
                .to_string(),
        ));
    }
    let fake_mode = fake_runtime;
    let docker_mode = args.iter().any(|arg| arg == "--docker-slot-provider");
    if fake_mode == docker_mode {
        return Err(LifecycleError::Usage(
            "run requires exactly one provider mode: --fake-runtime --fake-provider or --docker-slot-provider".to_string(),
        ));
    }
    if fake_mode
        && args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--docker-bin"
                    | "--artifact-container-root"
                    | "--live-send"
                    | "--require-visual-gate"
            )
        })
    {
        return Err(LifecycleError::Usage(
            "fake run forbids Docker options and live-send gates".to_string(),
        ));
    }
    Ok(fake_mode)
}

fn require_json(args: &[String]) -> Result<(), LifecycleError> {
    if args.iter().any(|arg| arg == "--json") {
        Ok(())
    } else {
        Err(LifecycleError::Usage("run requires --json".to_string()))
    }
}

fn require_live_docker_gate(args: &[String]) -> Result<(), LifecycleError> {
    if args.iter().any(|arg| arg == "--live-send")
        && args.iter().any(|arg| arg == "--require-visual-gate")
    {
        Ok(())
    } else {
        Err(LifecycleError::Usage(
            "docker run requires --live-send --require-visual-gate".to_string(),
        ))
    }
}

fn docker_bin(args: &[String]) -> PathBuf {
    option_value(args, "--docker-bin")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"))
}

fn artifact_expectation(args: &[String]) -> Result<ArtifactExpectation, LifecycleError> {
    match option_value(args, "--artifact-expectation") {
        Some(value) => ArtifactExpectation::parse(&value).ok_or_else(|| {
            LifecycleError::Usage(format!(
                "unsupported --artifact-expectation: {value}; expected none, optional, required, or claimed"
            ))
        }),
        None => Err(LifecycleError::Usage(
            "run requires --artifact-expectation".to_string(),
        )),
    }
}

fn model_effort(args: &[String]) -> Result<(String, String), LifecycleError> {
    let model = required_value(args, "--model")?;
    let effort = match (model.as_str(), option_value(args, "--effort").as_deref()) {
        ("pro", None | Some("standard")) => "standard",
        ("xhigh", None | Some("high")) => "high",
        ("pro", Some(other)) => {
            return Err(LifecycleError::Usage(format!(
                "model pro accepts only effort standard, got {other}"
            )))
        }
        ("xhigh", Some(other)) => {
            return Err(LifecycleError::Usage(format!(
                "model xhigh accepts only effort high, got {other}"
            )))
        }
        (other, _) => {
            return Err(LifecycleError::Usage(format!(
                "unsupported --model: {other}; expected pro or xhigh"
            )))
        }
    };
    Ok((model, effort.to_string()))
}

fn validate_docker_options(args: &[String]) -> Result<(), LifecycleError> {
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
