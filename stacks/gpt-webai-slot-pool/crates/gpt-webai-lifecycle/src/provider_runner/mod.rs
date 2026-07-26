use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::contracts::ids::{validate_operation_id, validate_request_key, validate_slot_id};
use crate::slots;

const ATTACHMENT_CONTAINER_ROOT: &str = "/broker-attachments";
const R13_ARTIFACT_CONTAINER_ROOT: &str = "/broker-artifacts";
const R13_PROVIDER_WORKDIR: &str = "/usr/local/lib/gpt-webai-slot-pool";
const R13_PROVIDER_ENTRYPOINT: &str = "provider/chatgpt-playwright/cli.mjs";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderExecution {
    Host(HostProviderExecution),
    DockerSlot(DockerSlotProviderExecution),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostProviderExecution {
    pub provider_bin: PathBuf,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerSlotProviderExecution {
    pub docker_bin: PathBuf,
    pub artifact_container_root: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCommand {
    pub provider_bin: PathBuf,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
    pub path_mode: ProviderPathMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct R13ProviderCommand {
    pub provider_bin: PathBuf,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
    pub slot_id: String,
    pub request_key: String,
    pub operation_id: String,
    pub paths: R13ProviderPaths,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct R13ProviderPaths {
    pub operation_host_dir: PathBuf,
    pub operation_container_dir: PathBuf,
    pub request_host_path: PathBuf,
    pub request_container_path: PathBuf,
    pub artifacts_host_dir: PathBuf,
    pub artifacts_container_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderPathMode {
    Host,
    DockerSlot(DockerSlotPaths),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerSlotPaths {
    pub artifact_host_dir: PathBuf,
    pub artifact_container_dir: String,
    pub attachment_host_dir: PathBuf,
    pub attachment_container_dir: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderCommandContext<'a> {
    pub config: &'a SupervisorConfig,
    pub slot_id: &'a str,
    pub run_id: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct R13ProviderCommandContext<'a> {
    pub config: &'a SupervisorConfig,
    pub slot_id: &'a str,
    pub request_key: &'a str,
    pub operation_id: &'a str,
}

#[derive(Debug, Error)]
pub enum ProviderRunnerError {
    #[error("slot not found for provider command: {0}")]
    SlotMissing(String),
    #[error("invalid R13 provider command identity: {0}")]
    InvalidIdentity(&'static str),
    #[error("provider environment name is not allowed: {0}")]
    EnvironmentNotAllowed(String),
    #[error("R13 Docker artifact container root must be /broker-artifacts")]
    InvalidDockerArtifactRoot,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl ProviderExecution {
    pub fn docker_bin(&self) -> Option<&Path> {
        match self {
            Self::DockerSlot(docker) => Some(&docker.docker_bin),
            Self::Host(_) => None,
        }
    }

    pub fn command(
        &self,
        context: ProviderCommandContext<'_>,
    ) -> Result<ProviderCommand, ProviderRunnerError> {
        match self {
            Self::Host(host) => Ok(ProviderCommand {
                provider_bin: host.provider_bin.clone(),
                args_prefix: host.args_prefix.clone(),
                env: host.env.clone(),
                path_mode: ProviderPathMode::Host,
            }),
            Self::DockerSlot(docker) => docker_command(docker, context),
        }
    }

    pub fn r13_command(
        &self,
        context: R13ProviderCommandContext<'_>,
    ) -> Result<R13ProviderCommand, ProviderRunnerError> {
        validate_r13_context(context)?;
        match self {
            Self::Host(host) => {
                let paths = r13_host_paths(context);
                create_private_directory(&context.config.state_root, &paths.operation_host_dir)?;
                create_private_directory(&context.config.state_root, &paths.artifacts_host_dir)?;
                validate_environment(&host.env)?;
                Ok(R13ProviderCommand {
                    provider_bin: host.provider_bin.clone(),
                    args_prefix: host.args_prefix.clone(),
                    env: host.env.clone(),
                    slot_id: context.slot_id.to_string(),
                    request_key: context.request_key.to_string(),
                    operation_id: context.operation_id.to_string(),
                    paths,
                })
            }
            Self::DockerSlot(docker) => r13_docker_command(docker, context),
        }
    }
}

fn validate_r13_context(context: R13ProviderCommandContext<'_>) -> Result<(), ProviderRunnerError> {
    validate_slot_id(context.slot_id)
        .map_err(|_| ProviderRunnerError::InvalidIdentity("slotId"))?;
    validate_request_key(context.request_key)
        .map_err(|_| ProviderRunnerError::InvalidIdentity("requestKey"))?;
    validate_operation_id(context.operation_id)
        .map_err(|_| ProviderRunnerError::InvalidIdentity("operationId"))?;
    if context.request_key.starts_with("d-")
        && context.request_key != format!("d-{}", context.operation_id)
    {
        return Err(ProviderRunnerError::InvalidIdentity(
            "diagnostic requestKey",
        ));
    }
    Ok(())
}

fn r13_host_paths(context: R13ProviderCommandContext<'_>) -> R13ProviderPaths {
    let operation_host_dir = context
        .config
        .state_root
        .join(r13_operation_relative_path(context));
    let artifacts_host_dir = context
        .config
        .state_root
        .join("artifacts")
        .join(context.request_key);
    let operation_container_dir = operation_host_dir.clone();
    let artifacts_container_dir = artifacts_host_dir.clone();
    R13ProviderPaths {
        request_host_path: operation_host_dir.join("provider-request.json"),
        request_container_path: operation_container_dir.join("provider-request.json"),
        operation_host_dir,
        operation_container_dir,
        artifacts_host_dir,
        artifacts_container_dir,
    }
}

fn r13_docker_command(
    docker: &DockerSlotProviderExecution,
    context: R13ProviderCommandContext<'_>,
) -> Result<R13ProviderCommand, ProviderRunnerError> {
    if docker.artifact_container_root.trim_end_matches('/') != R13_ARTIFACT_CONTAINER_ROOT {
        return Err(ProviderRunnerError::InvalidDockerArtifactRoot);
    }
    let slot = slots::inventory(context.config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == context.slot_id)
        .ok_or_else(|| ProviderRunnerError::SlotMissing(context.slot_id.to_string()))?;
    let relative_operation = r13_operation_relative_path(context);
    let operation_host_dir = context
        .config
        .state_root
        .join("slots")
        .join(context.slot_id)
        .join("state")
        .join(&relative_operation);
    let operation_container_dir =
        PathBuf::from(format!("/state/{}", context.slot_id)).join(&relative_operation);
    let artifacts_host_dir = context
        .config
        .state_root
        .join("artifacts")
        .join(context.request_key);
    let artifacts_container_dir =
        PathBuf::from(R13_ARTIFACT_CONTAINER_ROOT).join(context.request_key);
    let paths = R13ProviderPaths {
        request_host_path: operation_host_dir.join("provider-request.json"),
        request_container_path: operation_container_dir.join("provider-request.json"),
        operation_host_dir,
        operation_container_dir,
        artifacts_host_dir,
        artifacts_container_dir,
    };
    create_private_directory(&context.config.state_root, &paths.operation_host_dir)?;
    create_private_directory(&context.config.state_root, &paths.artifacts_host_dir)?;

    let uid = current_id("-u").unwrap_or_else(|| "1000".to_string());
    let gid = current_id("-g").unwrap_or_else(|| "1000".to_string());
    let mut provider_environment = vec![
        (
            "GPT_WEBAI_STATE_DIR".to_string(),
            format!("/state/{}", context.slot_id),
        ),
        (
            "GPT_WEBAI_ARTIFACTS_DIR".to_string(),
            paths.artifacts_container_dir.display().to_string(),
        ),
        (
            "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
            paths.artifacts_host_dir.display().to_string(),
        ),
    ];
    if let Some(name) = crate::failpoint::requested() {
        provider_environment.push((crate::failpoint::ENV_NAME.to_string(), name));
    }
    validate_environment(&provider_environment)?;

    let mut args_prefix = vec![
        "exec".to_string(),
        "-i".to_string(),
        "--user".to_string(),
        format!("{uid}:{gid}"),
        "--workdir".to_string(),
        R13_PROVIDER_WORKDIR.to_string(),
    ];
    for (name, value) in provider_environment {
        args_prefix.push("--env".to_string());
        args_prefix.push(format!("{name}={value}"));
    }
    args_prefix.extend([
        slot.container,
        "node".to_string(),
        R13_PROVIDER_ENTRYPOINT.to_string(),
    ]);

    Ok(R13ProviderCommand {
        provider_bin: docker.docker_bin.clone(),
        args_prefix,
        env: Vec::new(),
        slot_id: context.slot_id.to_string(),
        request_key: context.request_key.to_string(),
        operation_id: context.operation_id.to_string(),
        paths,
    })
}

fn r13_operation_relative_path(context: R13ProviderCommandContext<'_>) -> PathBuf {
    if context.request_key.starts_with("d-") {
        PathBuf::from("evidence")
            .join("diagnostics")
            .join(context.operation_id)
    } else {
        PathBuf::from("evidence")
            .join("requests")
            .join(context.request_key)
            .join("operations")
            .join(context.operation_id)
    }
}

pub(crate) fn create_private_directory(state_root: &Path, path: &Path) -> Result<(), io::Error> {
    if !state_root.is_absolute() || !path.starts_with(state_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "R13 provider directory is outside state root",
        ));
    }
    let mut directory = ensure_private_state_root(state_root)?;
    let relative = path
        .strip_prefix(state_root)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid state-root path"))?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid R13 provider path component",
            ));
        };
        mkdirat_private(&directory, name)?;
        directory = openat_directory(&directory, name)?;
        require_private_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn ensure_private_state_root(path: &Path) -> Result<File, io::Error> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state root must be absolute",
        ));
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::RootDir => None,
            std::path::Component::Normal(name) => Some(Ok(name)),
            _ => Some(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid state-root component",
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state root cannot be filesystem root",
        ));
    }

    let mut directory = open_root_directory()?;
    for (index, name) in components.iter().enumerate() {
        mkdirat_private(&directory, name)?;
        directory = openat_directory(&directory, name)?;
        if index + 1 == components.len() {
            require_private_directory(&directory)?;
        }
    }
    Ok(directory)
}

fn open_root_directory() -> Result<File, io::Error> {
    let root = CString::new("/").expect("root path has no NUL");
    let fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn mkdirat_private(directory: &File, name: &std::ffi::OsStr) -> Result<(), io::Error> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let result = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        Ok(())
    } else {
        Err(error)
    }
}

fn openat_directory(directory: &File, name: &std::ffi::OsStr) -> Result<File, io::Error> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn require_private_directory(directory: &File) -> Result<(), io::Error> {
    let metadata = directory.metadata()?;
    if metadata.is_dir() && metadata.mode() & 0o777 == 0o700 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "R13 provider directory must be mode 0700",
        ))
    }
}

fn validate_environment(environment: &[(String, String)]) -> Result<(), ProviderRunnerError> {
    const ALLOWED: [&str; 12] = [
        "LANG",
        "LC_ALL",
        "TZ",
        "DISPLAY",
        "CHROME_BINARY_PATH",
        "CHROME_NO_SANDBOX",
        "GPT_WEBAI_STATE_DIR",
        "GPT_WEBAI_ARTIFACTS_DIR",
        "GPT_WEBAI_ARTIFACTS_HOST_DIR",
        "GPT_WEBAI_FAKE_SCRIPT",
        "GPT_WEBAI_FAILPOINT",
        "GPT_WEBAI_TEST_EPOCH_MS",
    ];
    for (name, _) in environment {
        if !ALLOWED.contains(&name.as_str()) {
            return Err(ProviderRunnerError::EnvironmentNotAllowed(name.clone()));
        }
    }
    Ok(())
}

pub fn fake_provider_environment() -> Vec<(String, String)> {
    [
        "GPT_WEBAI_FAKE_SCRIPT",
        "GPT_WEBAI_FAILPOINT",
        "GPT_WEBAI_TEST_EPOCH_MS",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| (name.to_string(), value))
    })
    .collect()
}

fn docker_command(
    docker: &DockerSlotProviderExecution,
    context: ProviderCommandContext<'_>,
) -> Result<ProviderCommand, ProviderRunnerError> {
    let slot = slots::inventory(context.config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == context.slot_id)
        .ok_or_else(|| ProviderRunnerError::SlotMissing(context.slot_id.to_string()))?;
    let run_key = safe_key(context.run_id);
    let slot_state_root = context
        .config
        .state_root
        .join("slots")
        .join(context.slot_id);
    let artifact_host_dir = slot_state_root.join("artifacts").join(&run_key);
    let attachment_host_dir = slot_state_root.join("attachments").join(&run_key);
    let artifact_container_dir = format!(
        "{}/{}",
        docker.artifact_container_root.trim_end_matches('/'),
        run_key
    );
    let attachment_container_dir = format!("{ATTACHMENT_CONTAINER_ROOT}/{run_key}");
    create_private_directory(
        &context.config.state_root,
        &artifact_host_dir.join("downloads"),
    )?;
    create_private_directory(&context.config.state_root, &attachment_host_dir)?;
    let uid = current_id("-u").unwrap_or_else(|| "1000".to_string());
    let gid = current_id("-g").unwrap_or_else(|| "1000".to_string());

    Ok(ProviderCommand {
        provider_bin: docker.docker_bin.clone(),
        args_prefix: vec![
            "exec".to_string(),
            "-i".to_string(),
            "--user".to_string(),
            format!("{uid}:{gid}"),
            "--env".to_string(),
            format!("BROWSER_AGENT_HOME=/state/{}", slot.slot_id.0),
            "--env".to_string(),
            format!("CDP_PORT={}", slot.cdp_port),
            "--env".to_string(),
            format!("GPT_WEBAI_ARTIFACTS_DIR={artifact_container_dir}"),
            "--env".to_string(),
            format!(
                "GPT_WEBAI_ARTIFACTS_HOST_DIR={}",
                artifact_host_dir.display()
            ),
            slot.container,
            "gpt-webai-provider".to_string(),
        ],
        env: Vec::new(),
        path_mode: ProviderPathMode::DockerSlot(DockerSlotPaths {
            artifact_host_dir,
            artifact_container_dir,
            attachment_host_dir,
            attachment_container_dir,
        }),
    })
}

fn current_id(flag: &str) -> Option<String> {
    let output = Command::new("id").arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
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
