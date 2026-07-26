use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::provider_runner::{DockerSlotProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::runtime::{
    docker_runtime_for_provider, DockerStatus, ProviderReadiness, RuntimeProbe,
};
use gpt_webai_lifecycle::slots;
use serde_json::json;

#[test]
fn docker_runtime_uses_provider_execution_docker_bin_for_probe() {
    let fixture = Fixture::new("runtime-docker-bin");
    let docker = fixture.write_fake_docker();
    let config = fixture.config();
    let provider = ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: docker,
        artifact_container_root: "/broker-artifacts".to_string(),
    });
    let runtime = docker_runtime_for_provider(&config, &provider);
    let slot = slots::inventory(&config).remove(0);

    let observation = runtime.observe(&slot);

    assert_eq!(observation.docker_status, DockerStatus::Running);
    assert_eq!(observation.provider_readiness, ProviderReadiness::Ready);
    let log = fs::read_to_string(&fixture.log).expect("docker log");
    assert!(log.contains("inspect -f {{.State.Status}} gpt-webai-slot-01"));
    assert!(log.contains("exec -i"));
    assert!(log.contains("gpt-webai-provider status"));
}

struct Fixture {
    root: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let root = temp_root(prefix);
        fs::create_dir_all(&root).expect("root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
        let log = root.join("docker.log");
        Self { root, log }
    }

    fn config(&self) -> SupervisorConfig {
        SupervisorConfig {
            state_root: self.root.clone(),
            slot_count: 1,
            slot_container_prefix: "gpt-webai-".to_string(),
            slot_mode: "docker".to_string(),
            status_provider_check: true,
            provider_status_timeout_ms: 1_000,
        }
    }

    fn write_fake_docker(&self) -> PathBuf {
        let path = self.root.join("fake-docker.sh");
        let ready = json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "ready"
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$1\" in\n\
                 inspect) printf '%s\\n' running ;;\n\
                 exec) printf '%s\\n' '{}' ;;\n\
                 *) exit 2 ;;\n\
                 esac\n",
                self.log.display(),
                ready
            ),
        )
        .expect("write fake docker");
        set_executable(&path);
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_root(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "gpt-webai-runtime-{prefix}-{}-{nonce}",
        std::process::id()
    ))
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}
