use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::request::artifact_expectation::ArtifactExpectation;
use gpt_webai_lifecycle::request::run::RequestRunInput;
use gpt_webai_lifecycle::runtime::{
    control::{RuntimeReleaseMode, RuntimeStartMode},
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct FakeRun {
    root: PathBuf,
    pub provider: PathBuf,
    pub prompt_file: PathBuf,
    pub args_log: PathBuf,
}

pub struct InputSpec {
    pub send_json: PathBuf,
    pub poll_json: PathBuf,
    pub download_json: Option<PathBuf>,
    pub files: Vec<PathBuf>,
}

impl FakeRun {
    pub fn new(prefix: &str) -> Self {
        let root = temp_path(prefix);
        let provider = write_fake_provider(&root);
        let prompt_file = write_file(&root, "prompt.md", "hello");
        let args_log = root.join("args.log");
        Self {
            root,
            provider,
            prompt_file,
            args_log,
        }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn write_file(&self, name: &str, text: &str) -> PathBuf {
        write_file(&self.root, name, text)
    }

    pub fn write_json(&self, name: &str, value: serde_json::Value) -> PathBuf {
        self.write_file(name, &value.to_string())
    }

    pub fn write_fake_docker(&self, log_path: &Path, exit_code: u8) -> PathBuf {
        let path = self.root.join(format!("fake-docker-{exit_code}.sh"));
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" >> '{}'\nexit {exit_code}\n",
                log_path.display()
            ),
        )
        .expect("write fake docker");
        set_executable(&path);
        path
    }

    pub fn input(&self, spec: InputSpec) -> RequestRunInput {
        let download_artifacts_after_poll = spec.download_json.is_some();
        let mut env = vec![
            (
                "FAKE_PROVIDER_SEND_JSON".to_string(),
                spec.send_json.display().to_string(),
            ),
            (
                "FAKE_PROVIDER_POLL_JSON".to_string(),
                spec.poll_json.display().to_string(),
            ),
            (
                "FAKE_PROVIDER_ARGS_LOG".to_string(),
                self.args_log.display().to_string(),
            ),
        ];
        if let Some(download_json) = spec.download_json {
            env.push((
                "FAKE_PROVIDER_DOWNLOAD_JSON".to_string(),
                download_json.display().to_string(),
            ));
        }
        RequestRunInput {
            config: SupervisorConfig {
                state_root: self.root.clone(),
                slot_count: 2,
                slot_container_prefix: "gpt-webai-".to_string(),
                slot_mode: "docker".to_string(),
                status_provider_check: true,
                provider_status_timeout_ms: 1_000,
            },
            provider_execution: ProviderExecution::Host(HostProviderExecution {
                provider_bin: self.provider.clone(),
                args_prefix: Vec::new(),
                env,
            }),
            runtime_start_mode: RuntimeStartMode::Disabled,
            runtime_release_mode: RuntimeReleaseMode::LockOnly,
            pre_send_visual_gate: false,
            pre_poll_wait_gate: false,
            download_artifacts_after_poll,
            artifact_expectation: ArtifactExpectation::Optional,
            prompt_file: self.prompt_file.clone(),
            files: spec.files,
            request_id: "request-run".to_string(),
            run_id: "run-a".to_string(),
            fencing_token: "fence-a".to_string(),
            model: "pro".to_string(),
            effort: "extended".to_string(),
            ttl_ms: 30_000,
            send_retry_delays: Vec::new(),
            provider_limit_retry_delays: Vec::new(),
            send_process_timeout: Duration::from_secs(2),
            poll_timeout_seconds: 30,
            poll_process_timeout: Duration::from_secs(2),
            max_stdout_bytes: 16_384,
            max_stderr_bytes: 1_024,
        }
    }
}

impl Drop for FakeRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn ready_runtime() -> StaticRuntimeProbe {
    runtime_with(DockerStatus::Running, ProviderReadiness::Ready)
}

pub fn standby_exited_runtime() -> StaticRuntimeProbe {
    runtime_with(DockerStatus::Exited, ProviderReadiness::NotChecked)
}

fn runtime_with(
    docker_status: DockerStatus,
    provider_readiness: ProviderReadiness,
) -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([
        (
            "slot-01".to_string(),
            RuntimeObservation {
                docker_status: docker_status.clone(),
                cdp_reachable: Some(matches!(provider_readiness, ProviderReadiness::Ready)),
                provider_readiness: provider_readiness.clone(),
            },
        ),
        (
            "slot-02".to_string(),
            RuntimeObservation {
                docker_status,
                cdp_reachable: Some(matches!(provider_readiness, ProviderReadiness::Ready)),
                provider_readiness,
            },
        ),
    ])
}

fn temp_path(prefix: &str) -> PathBuf {
    let base = test_temp_base().join("gpt-webai-request-run");
    fs::create_dir_all(&base).expect("create request test temp base");
    let prefix = safe_temp_component(prefix);

    for attempt in 0..1000_u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = base.join(format!(
            "{prefix}-{}-{counter}-{now}-{attempt}",
            std::process::id()
        ));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create temp dir {}: {error}", candidate.display()),
        }
    }

    panic!("unable to allocate unique temp dir for {prefix}");
}

fn test_temp_base() -> PathBuf {
    std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.join("target").join("test-tmp"))
        })
        .unwrap_or_else(std::env::temp_dir)
}

fn safe_temp_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = safe.trim_matches('-');
    if trimmed.is_empty() {
        "fixture".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn write_file(root: &Path, name: &str, text: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, text).expect("write file");
    path
}

fn write_fake_provider(dir: &Path) -> PathBuf {
    let path = dir.join("fake-provider.sh");
    fs::write(
        &path,
        "#!/usr/bin/env bash\nprintf '%s ' \"$@\" >> \"$FAKE_PROVIDER_ARGS_LOG\"\nprintf '\\n' >> \"$FAKE_PROVIDER_ARGS_LOG\"\ncase \"$1\" in\n  send) cat \"$FAKE_PROVIDER_SEND_JSON\" ;;\n  poll) cat \"$FAKE_PROVIDER_POLL_JSON\" ;;\n  download) cat \"$FAKE_PROVIDER_DOWNLOAD_JSON\" ;;\n  capture)\n    if [[ -n \"${FAKE_PROVIDER_CAPTURE_SLEEP_SECONDS:-}\" ]]; then sleep \"$FAKE_PROVIDER_CAPTURE_SLEEP_SECONDS\"; fi\n    if [[ -n \"${FAKE_PROVIDER_CAPTURE_JSON:-}\" ]]; then cat \"$FAKE_PROVIDER_CAPTURE_JSON\"; else printf '{\"schema\":\"gpt-webai.provider.envelope.v2\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}\\n'; exit 2; fi ;;\n  *) printf '{\"schema\":\"gpt-webai.provider.envelope.v2\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}\\n'; exit 2 ;;\nesac\n",
    )
    .expect("write fake provider");
    set_executable(&path);
    path
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}
