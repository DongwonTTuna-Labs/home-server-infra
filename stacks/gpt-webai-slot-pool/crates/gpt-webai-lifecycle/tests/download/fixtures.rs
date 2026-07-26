use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::download::DownloadInput;
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        Self {
            root: temp_state_root(prefix),
        }
    }

    pub(super) fn input(&self, provider: PathBuf, session_id: &str) -> DownloadInput {
        DownloadInput {
            config: config(self.root.clone()),
            session_id: session_id.to_string(),
            fencing_token: "fixture-fence".to_string(),
            provider_execution: ProviderExecution::Host(HostProviderExecution {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
            }),
            runtime_start_mode: RuntimeStartMode::Disabled,
            runtime_release_mode: RuntimeReleaseMode::LockOnly,
            artifact_expectation: None,
            provider_timeout: Duration::from_secs(2),
            poll_timeout_seconds: 300,
            max_stdout_bytes: 16_384,
            max_stderr_bytes: 1_024,
        }
    }

    pub(super) fn write_session(&self, session_id: &str, slot_id: &str, _account_group: &str) {
        let record = new_session_record(NewSessionRecord {
            request_id: Some(format!("request-{session_id}")),
            run_id: Some(format!("run-{session_id}")),
            session_id: session_id.to_string(),
            conversation_url: format!("https://chatgpt.com/c/{session_id}"),
            slot_id: slot_id.to_string(),
            cohort: gpt_webai_lifecycle::allocator::cohort_of(slot_id)
                .expect("fixture slot cohort")
                .to_string(),
            page_binding_generation: 1,
        })
        .expect("new session");
        write_session_record(&self.root, &record).expect("write session");
    }

    pub(super) fn write_provider(&self, args_file: &Path, stdout: String) -> PathBuf {
        let path = self.root.join("provider.sh");
        fs::create_dir_all(&self.root).expect("root");
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{}'\n",
                args_file.display(),
                stdout
            ),
        )
        .expect("write provider");
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("chmod");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn config(state_root: PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        state_root,
        slot_count: 10,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 1_000,
    }
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-download-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
