use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::contracts::ids::{
    h256, sha256_hex, validate_operation_id, validate_receipt_id, validate_request_id,
    validate_run_id, validate_session_id, validate_timestamp_ms,
};
use crate::contracts::provider::{
    ProviderContractError as R13ProviderContractError, ProviderOperation as R13ProviderOperation,
    ProviderRequest as R13ProviderRequest, ProviderResponse as R13ProviderResponse,
};
use crate::journal::canonical::{canonical_bytes, parse_canonical};
use crate::provider_runner::R13ProviderCommand;
use crate::request::artifact_expectation::ArtifactExpectation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use crate::provider_contract::{
    validate_provider_envelope, ProviderContractError, ProviderEnvelopeSummary, PROVIDER_SCHEMA,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderOperation {
    Status,
    Capture {
        session_id: Option<String>,
        label: String,
    },
    Send {
        prompt_file: PathBuf,
        model: String,
        effort: String,
        files: Vec<PathBuf>,
    },
    Poll {
        session_id: String,
        timeout_seconds: u64,
        artifact_expectation: ArtifactExpectation,
    },
    SessionShow {
        session_id: String,
    },
    SessionResume {
        session_id: String,
    },
    Download {
        session_id: String,
        artifact_expectation: Option<ArtifactExpectation>,
    },
}

impl ProviderOperation {
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::Status => vec!["status".to_string()],
            Self::Capture { session_id, label } => {
                let mut args = vec!["capture".to_string(), "--label".to_string(), label.clone()];
                if let Some(session_id) = session_id {
                    args.push("--session".to_string());
                    args.push(session_id.clone());
                }
                args
            }
            Self::Send {
                prompt_file,
                model,
                effort,
                files,
            } => {
                let mut args = vec![
                    "send".to_string(),
                    "--prompt-file".to_string(),
                    prompt_file.display().to_string(),
                    "--model".to_string(),
                    model.clone(),
                    "--effort".to_string(),
                    effort.clone(),
                ];
                for file in files {
                    args.push("--file".to_string());
                    args.push(file.display().to_string());
                }
                args
            }
            Self::Poll {
                session_id,
                timeout_seconds,
                artifact_expectation,
            } => vec![
                "poll".to_string(),
                "--session".to_string(),
                session_id.clone(),
                "--timeout".to_string(),
                timeout_seconds.to_string(),
                "--artifact-expectation".to_string(),
                artifact_expectation.as_str().to_string(),
            ],
            Self::SessionShow { session_id } => session_args("show", session_id),
            Self::SessionResume { session_id } => session_args("resume", session_id),
            Self::Download {
                session_id,
                artifact_expectation,
            } => {
                let mut args = vec![
                    "download".to_string(),
                    "--session".to_string(),
                    session_id.clone(),
                ];
                if let Some(artifact_expectation) = artifact_expectation {
                    args.push("--artifact-expectation".to_string());
                    args.push(artifact_expectation.as_str().to_string());
                }
                args
            }
        }
    }
}

fn session_args(action: &str, session_id: &str) -> Vec<String> {
    vec![
        "sessions".to_string(),
        action.to_string(),
        "--session".to_string(),
        session_id.to_string(),
    ]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderInvocation {
    pub provider_bin: PathBuf,
    pub args_prefix: Vec<String>,
    pub operation: ProviderOperation,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderInvocationResult {
    pub exit_code: Option<i32>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub value: Value,
    pub summary: ProviderEnvelopeSummary,
}

#[derive(Clone, Debug)]
pub struct R13ProviderInvocation<'a> {
    pub command: &'a R13ProviderCommand,
    pub request: &'a R13ProviderRequest,
    pub state_root: &'a Path,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct R13ProviderInvocationResult {
    pub exit_code: i32,
    pub request_sha256: String,
    pub response: R13ProviderResponse,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub receipt_id: String,
    pub receipt_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct R13ReceiptEnvelope {
    created_at_ms: u64,
    operation: R13ProviderOperation,
    operation_id: String,
    payload: Value,
    receipt_id: String,
    request_id: Option<String>,
    run_id: Option<String>,
    schema: String,
    session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct R13StageReceiptEnvelope {
    created_at_ms: u64,
    operation: String,
    operation_id: String,
    payload: Value,
    receipt_id: String,
    request_id: Option<String>,
    run_id: Option<String>,
    schema: String,
    session_id: Option<String>,
}

const R13_RECEIPT_SCHEMA: &str = "pr72.receipt.r13.v1";

#[derive(Debug, Error)]
pub enum ProviderInvocationError {
    #[error("provider process timed out after {0:?}")]
    Timeout(Duration),
    #[error("provider stdout exceeded {limit} bytes: {actual}")]
    StdoutTooLarge { limit: usize, actual: usize },
    #[error("provider stderr exceeded {limit} bytes: {actual}")]
    StderrTooLarge { limit: usize, actual: usize },
    #[error("provider stdout was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("provider contract invalid: {0}")]
    Contract(#[from] ProviderContractError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum R13ProviderInvocationError {
    #[error("provider process timed out after {0:?}")]
    Timeout(Duration),
    #[error("provider stdout exceeded {limit} bytes: {actual}")]
    StdoutTooLarge { limit: usize, actual: usize },
    #[error("provider stderr exceeded {limit} bytes: {actual}")]
    StderrTooLarge { limit: usize, actual: usize },
    #[error("provider request contract invalid: {0}")]
    RequestContract(R13ProviderContractError),
    #[error("provider response contract invalid: {0}")]
    ResponseContract(R13ProviderContractError),
    #[error("provider stdout was not one canonical JSON object plus LF: {0}")]
    Canonical(serde_json::Error),
    #[error("provider rc {code} is not a valid R13 envelope result: {stderr}")]
    ProcessExit { code: i32, stderr: String },
    #[error("provider rc/envelope mismatch for rc {code}")]
    ExitEnvelopeMismatch { code: i32 },
    #[error("provider process terminated without an exit code")]
    MissingExitCode,
    #[error("immutable provider request collision: {0}")]
    RequestCollision(PathBuf),
    #[error("provider receipt invalid: {0}")]
    Receipt(&'static str),
    #[error("provider answer bytes invalid: {0}")]
    Answer(&'static str),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run_r13_provider_invocation(
    invocation: &R13ProviderInvocation<'_>,
) -> Result<R13ProviderInvocationResult, R13ProviderInvocationError> {
    invocation
        .request
        .validate()
        .map_err(R13ProviderInvocationError::RequestContract)?;
    validate_command_request_fence(invocation.command, invocation.request)?;
    validate_r13_provider_paths(invocation.state_root, invocation.command)?;
    let request_bytes =
        canonical_bytes(invocation.request).map_err(R13ProviderInvocationError::Canonical)?;
    if request_bytes.len() > 1_048_576 {
        return Err(R13ProviderInvocationError::StdoutTooLarge {
            limit: 1_048_576,
            actual: request_bytes.len(),
        });
    }
    write_immutable_request(
        &invocation.command.paths.operation_host_dir,
        &invocation.command.paths.request_host_path,
        &request_bytes,
        &invocation.request.identity.operation_id,
    )?;

    let mut command = Command::new(&invocation.command.provider_bin);
    command.env_clear();
    command.args(&invocation.command.args_prefix);
    command.arg("--request-file");
    command.arg(&invocation.command.paths.request_container_path);
    for (key, value) in &invocation.command.env {
        command.env(key, value);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = run_with_timeout(
        command,
        invocation.timeout,
        invocation.max_stdout_bytes,
        invocation.max_stderr_bytes,
    )
    .map_err(map_r13_process_error)?;
    if output.stdout_bytes > invocation.max_stdout_bytes {
        return Err(R13ProviderInvocationError::StdoutTooLarge {
            limit: invocation.max_stdout_bytes,
            actual: output.stdout_bytes,
        });
    }
    if output.stderr_bytes > invocation.max_stderr_bytes {
        return Err(R13ProviderInvocationError::StderrTooLarge {
            limit: invocation.max_stderr_bytes,
            actual: output.stderr_bytes,
        });
    }
    let code = output
        .status
        .code()
        .ok_or(R13ProviderInvocationError::MissingExitCode)?;
    crate::failpoint::propagate_provider_exit(code, &output.stdout, &output.stderr);
    if !matches!(code, 0 | 2 | 70 | 124) {
        return Err(R13ProviderInvocationError::ProcessExit {
            code,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    if !matches!(code, 0 | 124) {
        return if output.stdout.is_empty() {
            Err(R13ProviderInvocationError::ProcessExit {
                code,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        } else {
            Err(R13ProviderInvocationError::ExitEnvelopeMismatch { code })
        };
    }
    let value = parse_canonical(&output.stdout).map_err(R13ProviderInvocationError::Canonical)?;
    let response: R13ProviderResponse =
        serde_json::from_value(value).map_err(R13ProviderInvocationError::Canonical)?;
    response
        .validate_for(invocation.request)
        .map_err(R13ProviderInvocationError::ResponseContract)?;
    if code == 124 && !valid_timeout_envelope(invocation.request, &response) {
        return Err(R13ProviderInvocationError::ExitEnvelopeMismatch { code });
    }
    let receipt_id = reopen_validate_receipt(
        &invocation.command.paths.operation_host_dir,
        invocation.request,
        &response,
    )?;
    let mut receipt_ids = vec![receipt_id.clone()];
    receipt_ids.extend(reopen_validate_send_receipts(
        &invocation.command.paths.operation_host_dir,
        invocation.request,
        &response,
    )?);
    reopen_validate_poll_answer(
        &invocation.command.paths.artifacts_host_dir,
        invocation.request,
        &response,
    )?;
    reopen_validate_rebind_answer(
        &invocation.command.paths.artifacts_host_dir,
        invocation.request,
        &response,
    )?;
    reopen_validate_download(invocation.state_root, invocation.request, &response)?;
    crate::failpoint::hit("after-receipt-before-event");
    Ok(R13ProviderInvocationResult {
        exit_code: code,
        request_sha256: h256(&request_bytes),
        response,
        stdout_bytes: output.stdout_bytes,
        stderr_bytes: output.stderr_bytes,
        receipt_id,
        receipt_ids,
    })
}

pub fn run_provider_invocation(
    invocation: &ProviderInvocation,
) -> Result<ProviderInvocationResult, ProviderInvocationError> {
    let mut command = Command::new(&invocation.provider_bin);
    command.args(&invocation.args_prefix);
    command.args(invocation.operation.args());
    for (key, value) in &invocation.env {
        command.env(key, value);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = run_with_timeout(
        command,
        invocation.timeout,
        invocation.max_stdout_bytes,
        invocation.max_stderr_bytes,
    )?;
    let stdout_bytes = output.stdout_bytes;
    let stderr_bytes = output.stderr_bytes;
    if stdout_bytes > invocation.max_stdout_bytes {
        return Err(ProviderInvocationError::StdoutTooLarge {
            limit: invocation.max_stdout_bytes,
            actual: stdout_bytes,
        });
    }
    if stderr_bytes > invocation.max_stderr_bytes {
        return Err(ProviderInvocationError::StderrTooLarge {
            limit: invocation.max_stderr_bytes,
            actual: stderr_bytes,
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value = serde_json::from_str::<Value>(stdout.trim())?;
    let summary = validate_provider_envelope(&value)?;
    Ok(ProviderInvocationResult {
        exit_code: output.status.code(),
        stdout_bytes,
        stderr_bytes,
        value,
        summary,
    })
}

fn validate_command_request_fence(
    command: &R13ProviderCommand,
    request: &R13ProviderRequest,
) -> Result<(), R13ProviderInvocationError> {
    if command.slot_id != request.identity.slot_id
        || command.operation_id != request.identity.operation_id
    {
        return Err(R13ProviderInvocationError::RequestContract(
            R13ProviderContractError::Invalid("command identity"),
        ));
    }
    let key_matches = if let Some(request_id) = &request.identity.request_id {
        command.request_key == format!("r-{request_id}")
    } else if let Some(session_id) = &request.identity.session_id {
        command.request_key == format!("s-{session_id}")
    } else {
        command.request_key == format!("d-{}", request.identity.operation_id)
    };
    if !key_matches {
        return Err(R13ProviderInvocationError::RequestContract(
            R13ProviderContractError::Invalid("command requestKey"),
        ));
    }
    Ok(())
}

fn valid_timeout_envelope(request: &R13ProviderRequest, response: &R13ProviderResponse) -> bool {
    request.operation == R13ProviderOperation::Poll
        && response.ok
        && response.status == "running"
        && response
            .operation_data
            .get("pollState")
            .and_then(Value::as_str)
            == Some("running")
}

fn map_r13_process_error(error: ProviderInvocationError) -> R13ProviderInvocationError {
    match error {
        ProviderInvocationError::Timeout(timeout) => R13ProviderInvocationError::Timeout(timeout),
        ProviderInvocationError::StdoutTooLarge { limit, actual } => {
            R13ProviderInvocationError::StdoutTooLarge { limit, actual }
        }
        ProviderInvocationError::StderrTooLarge { limit, actual } => {
            R13ProviderInvocationError::StderrTooLarge { limit, actual }
        }
        ProviderInvocationError::Io(error) => R13ProviderInvocationError::Io(error),
        ProviderInvocationError::Json(_) | ProviderInvocationError::Contract(_) => {
            R13ProviderInvocationError::Receipt("unexpected legacy process error")
        }
    }
}

fn write_immutable_request(
    operation_root: &Path,
    target: &Path,
    bytes: &[u8],
    operation_id: &str,
) -> Result<(), R13ProviderInvocationError> {
    let directory = open_operation_root(operation_root)?;
    if target != operation_root.join("provider-request.json") {
        return Err(R13ProviderInvocationError::Receipt("request path"));
    }
    let target_name = std::ffi::OsStr::new("provider-request.json");
    match read_regular_at(&directory, target_name, Some(0o600)) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(R13ProviderInvocationError::RequestCollision(
                target.to_path_buf(),
            ));
        }
        Err(R13ProviderInvocationError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let temp_name = format!(".provider-request.json.{operation_id}.tmp");
    let temp_name = std::ffi::OsStr::new(&temp_name);
    let temp_path = operation_root.join(temp_name);
    let mut file = match create_regular_at(&directory, temp_name, 0o600) {
        Ok(file) => file,
        Err(R13ProviderInvocationError::Io(error))
            if error.kind() == io::ErrorKind::AlreadyExists =>
        {
            let existing = read_regular_at(&directory, temp_name, Some(0o600))?;
            if existing != bytes {
                return Err(R13ProviderInvocationError::RequestCollision(temp_path));
            }
            match renameat_noreplace(&directory, temp_name, target_name) {
                Ok(()) => {}
                Err(link_error) if link_error.kind() == io::ErrorKind::AlreadyExists => {
                    let target_bytes = read_regular_at(&directory, target_name, Some(0o600))?;
                    if target_bytes != bytes {
                        return Err(R13ProviderInvocationError::RequestCollision(
                            target.to_path_buf(),
                        ));
                    }
                    unlinkat_file(&directory, temp_name)?;
                }
                Err(error) => return Err(error.into()),
            }
            directory.sync_all()?;
            let reopened = read_regular_at(&directory, target_name, Some(0o600))?;
            if reopened != bytes {
                return Err(R13ProviderInvocationError::RequestCollision(
                    target.to_path_buf(),
                ));
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match renameat_noreplace(&directory, temp_name, target_name) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = read_regular_at(&directory, target_name, Some(0o600))?;
            if existing != bytes {
                return Err(R13ProviderInvocationError::RequestCollision(
                    target.to_path_buf(),
                ));
            }
            unlinkat_file(&directory, temp_name)?;
        }
        Err(error) => return Err(error.into()),
    }
    directory.sync_all()?;
    let reopened = read_regular_at(&directory, target_name, Some(0o600))?;
    if reopened != bytes || parse_canonical(&reopened).is_err() {
        return Err(R13ProviderInvocationError::RequestCollision(
            target.to_path_buf(),
        ));
    }
    Ok(())
}

fn validate_operation_root(path: &Path) -> Result<(), R13ProviderInvocationError> {
    open_operation_root(path).map(|_| ())
}

fn validate_r13_provider_paths(
    state_root: &Path,
    command: &R13ProviderCommand,
) -> Result<(), R13ProviderInvocationError> {
    open_operation_root(state_root)?;
    let relative_operation_root = if command.request_key.starts_with("d-") {
        PathBuf::from("evidence")
            .join("diagnostics")
            .join(&command.operation_id)
    } else {
        PathBuf::from("evidence")
            .join("requests")
            .join(&command.request_key)
            .join("operations")
            .join(&command.operation_id)
    };
    let expected_host_operation_root = state_root.join(&relative_operation_root);
    let expected_docker_operation_root = state_root
        .join("slots")
        .join(&command.slot_id)
        .join("state")
        .join(&relative_operation_root);
    let expected_container_operation_root =
        PathBuf::from(format!("/state/{}", command.slot_id)).join(&relative_operation_root);
    let expected_artifact_root = state_root.join("artifacts").join(&command.request_key);
    let host_mapping = command.paths.operation_host_dir == expected_host_operation_root
        && command.paths.operation_container_dir == expected_host_operation_root
        && command.paths.request_host_path
            == expected_host_operation_root.join("provider-request.json")
        && command.paths.request_container_path
            == expected_host_operation_root.join("provider-request.json")
        && command.paths.artifacts_host_dir == expected_artifact_root
        && command.paths.artifacts_container_dir == expected_artifact_root;
    let docker_mapping = command.paths.operation_host_dir == expected_docker_operation_root
        && command.paths.operation_container_dir == expected_container_operation_root
        && command.paths.request_host_path
            == expected_docker_operation_root.join("provider-request.json")
        && command.paths.request_container_path
            == expected_container_operation_root.join("provider-request.json")
        && command.paths.artifacts_host_dir == expected_artifact_root
        && command.paths.artifacts_container_dir
            == PathBuf::from("/broker-artifacts").join(&command.request_key);
    if !host_mapping && !docker_mapping {
        return Err(R13ProviderInvocationError::Receipt("provider paths"));
    }
    validate_operation_root(&command.paths.operation_host_dir)?;
    validate_operation_root(&command.paths.artifacts_host_dir)
}

fn read_regular_beneath(
    root: &Path,
    relative: &Path,
    expected_mode: Option<u32>,
) -> Result<Vec<u8>, R13ProviderInvocationError> {
    let (mut file, expected_len) = open_regular_beneath(root, relative, expected_mode)?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(expected_len)
            .map_err(|_| R13ProviderInvocationError::Receipt("file size"))?,
    );
    file.read_to_end(&mut bytes)?;
    if expected_len != bytes.len() as u64 {
        return Err(R13ProviderInvocationError::Receipt("file size"));
    }
    Ok(bytes)
}

fn verify_regular_beneath(
    root: &Path,
    relative: &Path,
    expected_mode: Option<u32>,
    expected_size: u64,
    expected_sha256: &str,
    require_nonempty: bool,
) -> Result<(), R13ProviderInvocationError> {
    let (mut file, actual_size) = open_regular_beneath(root, relative, expected_mode)?;
    if actual_size != expected_size || (require_nonempty && actual_size == 0) {
        return Err(R13ProviderInvocationError::Receipt("digest/size"));
    }
    let mut hasher = Sha256::new();
    let mut observed_size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        observed_size = observed_size
            .checked_add(read as u64)
            .ok_or(R13ProviderInvocationError::Receipt("file size"))?;
        if observed_size > expected_size {
            return Err(R13ProviderInvocationError::Receipt("digest/size"));
        }
        hasher.update(&buffer[..read]);
    }
    let observed_sha256 = format!("sha256:{:x}", hasher.finalize());
    if observed_size != expected_size || observed_sha256 != expected_sha256 {
        return Err(R13ProviderInvocationError::Receipt("digest/size"));
    }
    Ok(())
}

fn open_regular_beneath(
    root: &Path,
    relative: &Path,
    expected_mode: Option<u32>,
) -> Result<(File, u64), R13ProviderInvocationError> {
    validate_operation_root(root)?;
    if relative.is_absolute() {
        return Err(R13ProviderInvocationError::Receipt("relative path"));
    }

    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(R13ProviderInvocationError::Receipt("relative path"));
    }

    let mut directory = open_directory_nofollow(root)?;
    for component in &components[..components.len() - 1] {
        let next = openat_nofollow(
            &directory,
            component.as_os_str(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .map_err(|_| R13ProviderInvocationError::Receipt("path component"))?;
        if !next.metadata()?.is_dir() {
            return Err(R13ProviderInvocationError::Receipt("path component"));
        }
        directory = next;
    }
    let file = openat_nofollow(
        &directory,
        components.last().expect("non-empty components").as_os_str(),
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )
    .map_err(|_| R13ProviderInvocationError::Receipt("file metadata"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || expected_mode.is_some_and(|mode| metadata.mode() & 0o777 != mode)
    {
        return Err(R13ProviderInvocationError::Receipt("file metadata"));
    }
    Ok((file, metadata.len()))
}

fn open_directory_nofollow(path: &Path) -> Result<File, R13ProviderInvocationError> {
    if !path.is_absolute() {
        return Err(R13ProviderInvocationError::Receipt("operation root"));
    }
    let mut directory = open_root_directory()?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = openat_nofollow(
                    &directory,
                    name,
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
                .map_err(|_| R13ProviderInvocationError::Receipt("path component"))?;
                if !directory.metadata()?.is_dir() {
                    return Err(R13ProviderInvocationError::Receipt("path component"));
                }
            }
            _ => return Err(R13ProviderInvocationError::Receipt("operation root")),
        }
    }
    Ok(directory)
}

fn open_root_directory() -> Result<File, R13ProviderInvocationError> {
    let root = CString::new("/").expect("root path has no NUL");
    let fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_operation_root(path: &Path) -> Result<File, R13ProviderInvocationError> {
    let directory = open_directory_nofollow(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return Err(R13ProviderInvocationError::Receipt("operation root"));
    }
    Ok(directory)
}

fn read_regular_at(
    directory: &File,
    name: &std::ffi::OsStr,
    expected_mode: Option<u32>,
) -> Result<Vec<u8>, R13ProviderInvocationError> {
    let mut file = openat_nofollow(
        directory,
        name,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || expected_mode.is_some_and(|mode| metadata.mode() & 0o777 != mode)
    {
        return Err(R13ProviderInvocationError::Receipt("file metadata"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if metadata.len() != bytes.len() as u64 {
        return Err(R13ProviderInvocationError::Receipt("file size"));
    }
    Ok(bytes)
}

fn create_regular_at(
    directory: &File,
    name: &std::ffi::OsStr,
    mode: libc::mode_t,
) -> Result<File, R13ProviderInvocationError> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| R13ProviderInvocationError::Receipt("path NUL"))?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_CLOEXEC
                | libc::O_NOCTTY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn openat_nofollow(
    directory: &File,
    component: &std::ffi::OsStr,
    flags: libc::c_int,
) -> Result<File, R13ProviderInvocationError> {
    let component = CString::new(component.as_bytes())
        .map_err(|_| R13ProviderInvocationError::Receipt("path NUL"))?;
    let fd = unsafe { libc::openat(directory.as_raw_fd(), component.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn renameat_noreplace(
    directory: &File,
    source: &std::ffi::OsStr,
    target: &std::ffi::OsStr,
) -> Result<(), io::Error> {
    let source = CString::new(source.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = CString::new(target.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn unlinkat_file(directory: &File, name: &std::ffi::OsStr) -> Result<(), io::Error> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn reopen_validate_receipt(
    operation_root: &Path,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<String, R13ProviderInvocationError> {
    let bytes = read_regular_beneath(
        operation_root,
        Path::new(&request.evidence.receipt_rel_paths.primary),
        Some(0o600),
    )?;
    if bytes.len() as u64 != response.receipt.size_bytes || h256(&bytes) != response.receipt.sha256
    {
        return Err(R13ProviderInvocationError::Receipt("digest/size"));
    }
    let value = parse_canonical(&bytes)
        .map_err(|_| R13ProviderInvocationError::Receipt("canonical JSON"))?;
    let receipt: R13ReceiptEnvelope =
        serde_json::from_value(value).map_err(|_| R13ProviderInvocationError::Receipt("schema"))?;
    validate_receipt_identity(&receipt, request, response)?;
    let mut blank = receipt.clone();
    blank.receipt_id.clear();
    let preimage =
        canonical_bytes(&blank).map_err(|_| R13ProviderInvocationError::Receipt("preimage"))?;
    let expected_id = format!("receipt_{}", sha256_hex(preimage));
    if receipt.receipt_id != expected_id {
        return Err(R13ProviderInvocationError::Receipt("receiptId"));
    }
    Ok(receipt.receipt_id)
}

fn validate_receipt_identity(
    receipt: &R13ReceiptEnvelope,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<(), R13ProviderInvocationError> {
    let valid = receipt.schema == R13_RECEIPT_SCHEMA
        && receipt.operation == request.operation
        && receipt.operation_id == request.identity.operation_id
        && receipt.request_id == request.identity.request_id
        && receipt.run_id == request.identity.run_id
        && receipt.session_id == request.identity.session_id
        && receipt.payload == response.operation_data
        && validate_receipt_id(&receipt.receipt_id).is_ok()
        && validate_operation_id(&receipt.operation_id).is_ok()
        && receipt
            .request_id
            .as_deref()
            .map(validate_request_id)
            .transpose()
            .is_ok()
        && receipt
            .run_id
            .as_deref()
            .map(validate_run_id)
            .transpose()
            .is_ok()
        && receipt
            .session_id
            .as_deref()
            .map(validate_session_id)
            .transpose()
            .is_ok()
        && validate_timestamp_ms(receipt.created_at_ms).is_ok();
    if valid {
        Ok(())
    } else {
        Err(R13ProviderInvocationError::Receipt("identity/payload"))
    }
}

fn reopen_validate_send_receipts(
    operation_root: &Path,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<Vec<String>, R13ProviderInvocationError> {
    if !matches!(
        request.operation,
        R13ProviderOperation::SendClick | R13ProviderOperation::SendReconcile
    ) {
        return Ok(Vec::new());
    }
    let data = response
        .operation_data
        .as_object()
        .ok_or(R13ProviderInvocationError::Receipt("send operationData"))?;
    let mut ids = Vec::new();
    if request.operation == R13ProviderOperation::SendClick {
        let pre_click = data
            .get("preClickReceipt")
            .ok_or(R13ProviderInvocationError::Receipt("preClickReceipt"))?;
        let pre_path = request
            .evidence
            .receipt_rel_paths
            .pre_click
            .as_deref()
            .ok_or(R13ProviderInvocationError::Receipt("preClick path"))?;
        ids.push(reopen_validate_stage_receipt(
            operation_root,
            pre_path,
            "send.pre_click",
            pre_click,
            request,
        )?);
    }
    let terminal = data
        .get("terminalSendReceipt")
        .ok_or(R13ProviderInvocationError::Receipt("terminalSendReceipt"))?;
    if terminal.is_null() {
        return Ok(ids);
    }
    let (path, operation) = match request.operation {
        R13ProviderOperation::SendClick => (
            request.evidence.receipt_rel_paths.post_click.as_deref(),
            "send.post_click",
        ),
        R13ProviderOperation::SendReconcile => (
            request.evidence.receipt_rel_paths.reconcile.as_deref(),
            "send.reconcile",
        ),
        _ => unreachable!(),
    };
    ids.push(reopen_validate_stage_receipt(
        operation_root,
        path.ok_or(R13ProviderInvocationError::Receipt("terminal path"))?,
        operation,
        terminal,
        request,
    )?);
    Ok(ids)
}

fn reopen_validate_stage_receipt(
    operation_root: &Path,
    rel_path: &str,
    expected_operation: &str,
    expected_payload: &Value,
    request: &R13ProviderRequest,
) -> Result<String, R13ProviderInvocationError> {
    let bytes = read_regular_beneath(operation_root, Path::new(rel_path), Some(0o600))?;
    let value = parse_canonical(&bytes)
        .map_err(|_| R13ProviderInvocationError::Receipt("stage canonical JSON"))?;
    let receipt: R13StageReceiptEnvelope = serde_json::from_value(value)
        .map_err(|_| R13ProviderInvocationError::Receipt("stage schema"))?;
    let payload_session_id = expected_payload
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let valid = receipt.schema == R13_RECEIPT_SCHEMA
        && receipt.operation == expected_operation
        && receipt.operation_id == request.identity.operation_id
        && receipt.request_id == request.identity.request_id
        && receipt.run_id == request.identity.run_id
        && receipt.session_id == payload_session_id
        && receipt.payload == *expected_payload
        && validate_receipt_id(&receipt.receipt_id).is_ok()
        && validate_timestamp_ms(receipt.created_at_ms).is_ok();
    if !valid {
        return Err(R13ProviderInvocationError::Receipt(
            "stage identity/payload",
        ));
    }
    let mut blank = receipt.clone();
    blank.receipt_id.clear();
    let preimage = canonical_bytes(&blank)
        .map_err(|_| R13ProviderInvocationError::Receipt("stage preimage"))?;
    if receipt.receipt_id != format!("receipt_{}", sha256_hex(preimage)) {
        return Err(R13ProviderInvocationError::Receipt("stage receiptId"));
    }
    Ok(receipt.receipt_id)
}

fn reopen_validate_poll_answer(
    operation_root: &Path,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<(), R13ProviderInvocationError> {
    if request.operation != R13ProviderOperation::Poll || !response.ok {
        return Ok(());
    }
    let Some(data) = response.operation_data.as_object() else {
        return Err(R13ProviderInvocationError::Answer("operationData"));
    };
    if data.get("pollState").and_then(Value::as_str) != Some("terminal") {
        return Ok(());
    }
    let rel_path = data
        .get("answerRelPath")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Answer("answerRelPath"))?;
    let expected_size = data
        .get("answerSizeBytes")
        .and_then(Value::as_u64)
        .ok_or(R13ProviderInvocationError::Answer("answerSizeBytes"))?;
    let expected_sha = data
        .get("answerSha256")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Answer("answerSha256"))?;
    verify_answer_beneath(
        operation_root,
        Path::new(rel_path),
        expected_size,
        expected_sha,
    )
}

fn reopen_validate_rebind_answer(
    operation_root: &Path,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<(), R13ProviderInvocationError> {
    if request.operation != R13ProviderOperation::SessionRebind || !response.ok {
        return Ok(());
    }
    let Some(answer) = response.operation_data.get("terminalAnswer") else {
        return Err(R13ProviderInvocationError::Answer("terminalAnswer"));
    };
    if answer.is_null() {
        return Ok(());
    }
    reopen_validate_answer_object(operation_root, answer)
}

fn reopen_validate_answer_object(
    operation_root: &Path,
    answer: &Value,
) -> Result<(), R13ProviderInvocationError> {
    let rel_path = answer
        .get("answerRelPath")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Answer("answerRelPath"))?;
    let expected_size = answer
        .get("answerSizeBytes")
        .and_then(Value::as_u64)
        .ok_or(R13ProviderInvocationError::Answer("answerSizeBytes"))?;
    let expected_sha = answer
        .get("answerSha256")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Answer("answerSha256"))?;
    verify_answer_beneath(
        operation_root,
        Path::new(rel_path),
        expected_size,
        expected_sha,
    )
}

fn reopen_validate_download(
    artifacts_host_root: &Path,
    request: &R13ProviderRequest,
    response: &R13ProviderResponse,
) -> Result<(), R13ProviderInvocationError> {
    if request.operation != R13ProviderOperation::ArtifactClickSave || !response.ok {
        return Ok(());
    }
    let receipt = response
        .operation_data
        .get("downloadReceipt")
        .ok_or(R13ProviderInvocationError::Receipt("downloadReceipt"))?;
    let rel_path = receipt
        .get("hostSavedRelPath")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Receipt(
            "download hostSavedRelPath",
        ))?;
    let expected_size = receipt
        .get("sizeBytes")
        .and_then(Value::as_u64)
        .ok_or(R13ProviderInvocationError::Receipt("download sizeBytes"))?;
    let expected_sha = receipt
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or(R13ProviderInvocationError::Receipt("download sha256"))?;
    verify_regular_beneath(
        artifacts_host_root,
        Path::new(rel_path),
        Some(0o600),
        expected_size,
        expected_sha,
        true,
    )
    .map_err(|error| match error {
        R13ProviderInvocationError::Receipt("digest/size") => {
            R13ProviderInvocationError::Receipt("download digest/size")
        }
        other => other,
    })
}

fn verify_answer_beneath(
    root: &Path,
    relative: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), R13ProviderInvocationError> {
    verify_regular_beneath(
        root,
        relative,
        Some(0o600),
        expected_size,
        expected_sha256,
        false,
    )
    .map_err(|error| match error {
        R13ProviderInvocationError::Receipt("digest/size") => {
            R13ProviderInvocationError::Answer("digest/size")
        }
        other => other,
    })
}

#[derive(Debug)]
struct ProviderProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stdout_bytes: usize,
    stderr: Vec<u8>,
    stderr_bytes: usize,
}

#[derive(Debug)]
struct StreamCapture {
    bytes: Vec<u8>,
    actual_bytes: usize,
}

fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<ProviderProcessOutput, ProviderInvocationError> {
    let deadline = Instant::now() + timeout;
    let mut child = spawn_with_retry(&mut command, deadline)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("provider stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("provider stderr pipe missing"))?;
    let stdout_reader = thread::spawn(move || drain_stream(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || drain_stream(stderr, stderr_limit));

    loop {
        if child.try_wait()?.is_some() {
            let status = child.wait()?;
            let stdout = join_stream_reader(stdout_reader)?;
            let stderr = join_stream_reader(stderr_reader)?;
            return Ok(ProviderProcessOutput {
                status,
                stdout: stdout.bytes,
                stdout_bytes: stdout.actual_bytes,
                stderr: stderr.bytes,
                stderr_bytes: stderr.actual_bytes,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_stream_reader(stdout_reader);
            let _ = join_stream_reader(stderr_reader);
            return Err(ProviderInvocationError::Timeout(timeout));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn drain_stream<R: Read>(mut reader: R, limit: usize) -> io::Result<StreamCapture> {
    let stored_limit = limit.saturating_add(1);
    let mut bytes = Vec::with_capacity(stored_limit.min(8192));
    let mut actual_bytes = 0_usize;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        actual_bytes = actual_bytes.saturating_add(read);
        let remaining = stored_limit.saturating_sub(bytes.len());
        if remaining > 0 {
            bytes.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    Ok(StreamCapture {
        bytes,
        actual_bytes,
    })
}

fn join_stream_reader(
    reader: thread::JoinHandle<io::Result<StreamCapture>>,
) -> Result<StreamCapture, ProviderInvocationError> {
    match reader.join() {
        Ok(result) => result.map_err(ProviderInvocationError::Io),
        Err(_) => Err(ProviderInvocationError::Io(io::Error::other(
            "provider output reader thread panicked",
        ))),
    }
}

fn spawn_with_retry(
    command: &mut Command,
    deadline: Instant,
) -> Result<Child, ProviderInvocationError> {
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error) if error.raw_os_error() == Some(26) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}
