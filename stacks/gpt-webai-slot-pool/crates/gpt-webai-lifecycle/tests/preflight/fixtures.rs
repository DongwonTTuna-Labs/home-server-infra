use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::preflight::PreflightInput;
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use serde_json::{json, Value};

pub(super) struct Fixture {
    root: PathBuf,
    provider: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        Self {
            root: temp_state_root(prefix),
            provider: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider")
                .canonicalize()
                .expect("canonical R13 fake provider"),
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.root
    }

    pub(super) fn input(&self, slot_id: Option<String>, frames: &[Value]) -> PreflightInput {
        let env = if frames.is_empty() {
            Vec::new()
        } else {
            vec![(
                "GPT_WEBAI_FAKE_SCRIPT".to_string(),
                self.write_script(frames).display().to_string(),
            )]
        };
        PreflightInput {
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
            slot_id,
            run_id: "preflight-run".to_string(),
            provider_timeout: Duration::from_secs(2),
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 262_144,
        }
    }

    pub(super) fn malformed_input(&self, slot_id: Option<String>) -> PreflightInput {
        let script = self.root.join("fake-script.json");
        write_private_json(
            &script,
            &json!([{
                "expectOperation":"status",
                "frame":null,
                "malformedBytesB64":"bm90LWpzb24K"
            }]),
        );
        let mut input = self.input(slot_id, &[]);
        input.provider_execution = ProviderExecution::Host(HostProviderExecution {
            provider_bin: self.provider.clone(),
            args_prefix: Vec::new(),
            env: vec![(
                "GPT_WEBAI_FAKE_SCRIPT".to_string(),
                script.display().to_string(),
            )],
        });
        input
    }

    fn write_script(&self, frames: &[Value]) -> PathBuf {
        let entries = frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let frame_path = self.root.join(format!("frame-{index}.json"));
                write_private_json(&frame_path, frame);
                json!({
                    "expectOperation":"status",
                    "frame":frame_path,
                    "malformedBytesB64":null
                })
            })
            .collect::<Vec<_>>();
        let script = self.root.join("fake-script.json");
        write_private_json(&script, &Value::Array(entries));
        script
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn ready_runtime() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([ready_slot("slot-01")])
}

pub(super) fn runtime_with_exited_slot() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([exited_slot("slot-01")])
}

pub(super) fn runtime_with_standby_first_ready_second() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([exited_slot("slot-01"), ready_slot("slot-02")])
}

fn ready_slot(slot_id: &str) -> (String, RuntimeObservation) {
    (
        slot_id.to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Running,
            cdp_reachable: Some(true),
            provider_readiness: ProviderReadiness::Ready,
        },
    )
}

fn exited_slot(slot_id: &str) -> (String, RuntimeObservation) {
    (
        slot_id.to_string(),
        RuntimeObservation {
            docker_status: DockerStatus::Exited,
            cdp_reachable: None,
            provider_readiness: ProviderReadiness::NotChecked,
        },
    )
}

fn write_private_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).expect("fixture json")).expect("write fixture json");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("fixture mode");
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-preflight-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
