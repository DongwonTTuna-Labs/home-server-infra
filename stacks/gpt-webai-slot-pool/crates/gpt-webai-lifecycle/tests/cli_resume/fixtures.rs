use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};
use serde_json::json;

pub(super) fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

pub(super) fn stdout_json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).expect("stdout json")
}

pub(super) fn done_resume(session_id: &str) -> String {
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

pub(super) struct Fixture {
    pub(super) root: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        let root = temp_state_root(prefix);
        Self { root }
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
        let stdout_file = self.root.join("provider-output.json");
        fs::write(&stdout_file, stdout).expect("provider output");
        let path = self.root.join("provider.sh");
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > '{}'\ncat '{}'\n",
                args_file.display(),
                stdout_file.display()
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

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-cli-resume-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
