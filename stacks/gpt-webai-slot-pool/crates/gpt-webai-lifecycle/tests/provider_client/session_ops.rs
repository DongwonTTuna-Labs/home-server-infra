use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::provider_client::{
    run_provider_invocation, ProviderInvocation, ProviderOperation, PROVIDER_SCHEMA,
};
use gpt_webai_lifecycle::request::artifact_expectation::ArtifactExpectation;
use serde_json::json;

#[test]
fn runs_session_show_resume_and_download_provider_operations() {
    let dir = TestDir::new("session-ops");
    let fake_provider = write_fake_provider(dir.path());
    let args_file = dir.path().join("args.txt");
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "show",
        "sessionId": "sid-session",
        "conversationUrl": "https://chatgpt.com/c/sid-session"
    });
    let cases = [
        (
            ProviderOperation::SessionShow {
                session_id: "sid-session".to_string(),
            },
            "sessions\nshow\n--session\nsid-session\n",
        ),
        (
            ProviderOperation::SessionResume {
                session_id: "sid-session".to_string(),
            },
            "sessions\nresume\n--session\nsid-session\n",
        ),
        (
            ProviderOperation::Download {
                session_id: "sid-session".to_string(),
                artifact_expectation: None,
            },
            "download\n--session\nsid-session\n",
        ),
        (
            ProviderOperation::Download {
                session_id: "sid-session".to_string(),
                artifact_expectation: Some(ArtifactExpectation::Required),
            },
            "download\n--session\nsid-session\n--artifact-expectation\nrequired\n",
        ),
    ];

    for (operation, expected_args) in cases {
        let result = run_provider_invocation(&ProviderInvocation {
            provider_bin: fake_provider.clone(),
            args_prefix: Vec::new(),
            operation,
            env: fake_provider_env(&args_file, &stdout.to_string()),
            timeout: Duration::from_secs(2),
            max_stdout_bytes: 16_384,
            max_stderr_bytes: 1_024,
        })
        .expect("session provider invocation");

        assert_eq!(result.summary.session_id.as_deref(), Some("sid-session"));
        assert_eq!(fs::read_to_string(&args_file).expect("args"), expected_args);
    }
}

fn fake_provider_env(args_file: &Path, stdout: &str) -> Vec<(String, String)> {
    vec![
        (
            "FAKE_PROVIDER_ARGS_FILE".to_string(),
            args_file.display().to_string(),
        ),
        ("FAKE_PROVIDER_STDOUT".to_string(), stdout.to_string()),
    ]
}

fn write_fake_provider(dir: &Path) -> PathBuf {
    let path = dir.join("fake-provider.sh");
    fs::write(
        &path,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$FAKE_PROVIDER_ARGS_FILE\"\nprintf '%s\\n' \"$FAKE_PROVIDER_STDOUT\"\n",
    )
    .expect("write fake provider");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).expect("chmod");
    path
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gpt-webai-provider-session-ops-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
