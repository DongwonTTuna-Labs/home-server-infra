use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};
use serde_json::{json, Value};

pub(super) fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

pub(super) fn stdout_json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).expect("stdout json")
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

    pub(super) fn provider_bin(&self) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider")
            .canonicalize()
            .expect("canonical R13 fake provider")
    }

    pub(super) fn write_provider_script(&self, operations: &[&str]) -> PathBuf {
        let script = operations
            .iter()
            .map(|operation| {
                json!({
                    "expectOperation": operation,
                    "frame": Value::Null,
                    "malformedBytesB64": Value::Null,
                })
            })
            .collect::<Vec<_>>();
        let path = self.root.join("provider-script.json");
        let mut bytes = serde_json::to_vec(&script).expect("provider script json");
        bytes.push(b'\n');
        fs::write(&path, bytes).expect("provider script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("private provider script");
        path
    }

    pub(super) fn provider_invocation_count(&self, script: &Path) -> usize {
        let counter = PathBuf::from(format!("{}.counter", script.display()));
        match fs::read_to_string(counter) {
            Ok(value) => value.parse().expect("provider invocation counter"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("read provider invocation counter: {error}"),
        }
    }

    pub(super) fn provider_requests(&self) -> Vec<Value> {
        let mut paths = Vec::new();
        collect_named_files(&self.root, "provider-request.json", &mut paths);
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                serde_json::from_slice(&fs::read(&path).expect("provider request bytes"))
                    .expect("provider request json")
            })
            .collect()
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
        "gpt-webai-cli-download-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}

fn collect_named_files(root: &Path, name: &str, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read fixture tree") {
        let entry = entry.expect("fixture tree entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("fixture tree file type");
        if file_type.is_dir() {
            collect_named_files(&path, name, output);
        } else if file_type.is_file() && entry.file_name() == name {
            output.push(path);
        }
    }
}
