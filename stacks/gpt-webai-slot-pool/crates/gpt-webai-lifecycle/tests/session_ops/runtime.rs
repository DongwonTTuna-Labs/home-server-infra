use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::download::{download_session_with_runtime, DownloadInput};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::provider_runner::{HostProviderExecution, ProviderExecution};
use gpt_webai_lifecycle::resume::{resume_session_with_runtime, ResumeInput};
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};
use serde_json::json;

use super::fixtures::{exited_then_ready_runtime, standby_exited_runtime};

#[test]
fn resume_starts_standby_pinned_runtime_before_provider_and_stops_after_terminal_answer() {
    let fixture = Fixture::new("resume-starts");
    fixture.write_session("sid-resume-runtime");
    let provider = fixture.write_provider(done_resume("sid-resume-runtime"));
    let docker = fixture.write_fake_docker(0);

    let output = resume_session_with_runtime(
        fixture.resume_input(provider, docker, "sid-resume-runtime"),
        &exited_then_ready_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert!(output.runtime_started);
    assert!(output.runtime_owned);
    assert!(output.runtime_stopped);
    assert!(output.slot_state_written);
    assert_eq!(
        fs::read_to_string(&fixture.events).expect("events"),
        "docker start gpt-webai-slot-01\nprovider sessions resume --session sid-resume-runtime\ndocker stop gpt-webai-slot-01\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("slots").join("slot-01.state")).expect("slot state"),
        "status=standby\n"
    );
}

#[test]
fn download_starts_standby_pinned_runtime_before_provider_and_stops_after_attempt() {
    let fixture = Fixture::new("download-starts");
    fixture.write_session("sid-download-runtime");
    let provider = fixture.write_provider(done_download("sid-download-runtime"));
    let docker = fixture.write_fake_docker(0);

    let output = download_session_with_runtime(
        fixture.download_input(provider, docker, "sid-download-runtime"),
        &exited_then_ready_runtime(),
    );

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert!(output.runtime_started);
    assert!(output.runtime_owned);
    assert!(output.runtime_stopped);
    assert!(output.slot_state_written);
    assert_eq!(
        fs::read_to_string(&fixture.events).expect("events"),
        "docker start gpt-webai-slot-01\nprovider download --session sid-download-runtime\ndocker stop gpt-webai-slot-01\n"
    );
}

#[test]
fn resume_runtime_start_failure_does_not_invoke_provider() {
    let fixture = Fixture::new("resume-start-fails");
    fixture.write_session("sid-start-fails");
    let provider = fixture.write_provider(done_resume("sid-start-fails"));
    let docker = fixture.write_fake_docker(9);

    let output = resume_session_with_runtime(
        fixture.resume_input(provider, docker, "sid-start-fails"),
        &standby_exited_runtime(),
    );

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("runtime.start_failed"));
    assert!(!output.runtime_started);
    assert!(!output.runtime_owned);
    assert!(!output.runtime_stopped);
    assert_eq!(
        fs::read_to_string(&fixture.events).expect("events"),
        "docker start gpt-webai-slot-01\n"
    );
}

struct Fixture {
    root: PathBuf,
    events: PathBuf,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let root = temp_state_root(prefix);
        let events = root.join("events.log");
        Self { root, events }
    }

    fn resume_input(&self, provider: PathBuf, docker: PathBuf, session_id: &str) -> ResumeInput {
        ResumeInput {
            config: config(self.root.clone()),
            session_id: session_id.to_string(),
            fencing_token: "fixture-fence".to_string(),
            provider_execution: ProviderExecution::Host(HostProviderExecution {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
            }),
            runtime_start_mode: RuntimeStartMode::docker(docker.clone(), Duration::from_secs(2)),
            runtime_release_mode: RuntimeReleaseMode::docker(docker, Duration::from_secs(2)),
            provider_timeout: Duration::from_secs(2),
            poll_timeout_seconds: 300,
            max_stdout_bytes: 16_384,
            max_stderr_bytes: 1_024,
        }
    }

    fn download_input(
        &self,
        provider: PathBuf,
        docker: PathBuf,
        session_id: &str,
    ) -> DownloadInput {
        DownloadInput {
            config: config(self.root.clone()),
            session_id: session_id.to_string(),
            fencing_token: "fixture-fence".to_string(),
            provider_execution: ProviderExecution::Host(HostProviderExecution {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
            }),
            runtime_start_mode: RuntimeStartMode::docker(docker.clone(), Duration::from_secs(2)),
            runtime_release_mode: RuntimeReleaseMode::docker(docker, Duration::from_secs(2)),
            artifact_expectation: None,
            provider_timeout: Duration::from_secs(2),
            poll_timeout_seconds: 300,
            max_stdout_bytes: 16_384,
            max_stderr_bytes: 1_024,
        }
    }

    fn write_session(&self, session_id: &str) {
        let record = new_session_record(NewSessionRecord {
            request_id: Some(format!("request-{session_id}")),
            run_id: Some(format!("run-{session_id}")),
            session_id: session_id.to_string(),
            conversation_url: format!("https://chatgpt.com/c/{session_id}"),
            slot_id: "slot-01".to_string(),
            cohort: "cohort-a".to_string(),
            page_binding_generation: 1,
        })
        .expect("new session");
        write_session_record(&self.root, &record).expect("write session");
    }

    fn write_provider(&self, stdout: String) -> PathBuf {
        let stdout_file = self.root.join("provider-output.json");
        fs::write(&stdout_file, stdout).expect("provider output");
        let path = self.root.join("provider.sh");
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\nprintf 'provider %s\\n' \"$*\" >> '{}'\ncat '{}'\n",
                self.events.display(),
                stdout_file.display()
            ),
        )
        .expect("provider");
        set_executable(&path);
        path
    }

    fn write_fake_docker(&self, exit_code: u8) -> PathBuf {
        let path = self.root.join(format!("docker-{exit_code}.sh"));
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\nprintf 'docker %s\\n' \"$*\" >> '{}'\nexit {exit_code}\n",
                self.events.display()
            ),
        )
        .expect("docker");
        set_executable(&path);
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn done_resume(session_id: &str) -> String {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-resume",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    })
    .to_string()
}

fn done_download(session_id: &str) -> String {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "artifacts": [],
        "artifactCandidates": []
    })
    .to_string()
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

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-session-runtime-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
