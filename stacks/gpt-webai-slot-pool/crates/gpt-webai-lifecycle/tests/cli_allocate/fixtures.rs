use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

pub(super) fn stdout_json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).expect("stdout json")
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) docker_log: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        let root = temp_root(prefix);
        fs::create_dir(&root).expect("root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
        let docker_log = root.join("docker.log");
        Self { root, docker_log }
    }

    pub(super) fn write_slot_state(&self, slot_id: &str, status: &str) {
        let path = self.root.join("slots").join(format!("{slot_id}.state"));
        let parent = path.parent().expect("state parent");
        if !parent.exists() {
            fs::create_dir(parent).expect("state parent");
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .expect("private state parent");
        }
        fs::write(&path, format!("status={status}\n")).expect("slot state");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private slot state");
    }

    pub(super) fn slot_state(&self, slot_id: &str) -> String {
        fs::read_to_string(self.root.join("slots").join(format!("{slot_id}.state")))
            .expect("slot state")
    }

    pub(super) fn write_fake_docker(&self, inspect_status: &str, start_exit_code: u8) -> PathBuf {
        let path = self
            .root
            .join(format!("fake-docker-{inspect_status}-{start_exit_code}.sh"));
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$1\" in\n\
                 inspect) printf '%s\\n' '{inspect_status}' ;;\n\
                 start) exit {start_exit_code} ;;\n\
                 stop) exit 0 ;;\n\
                 *) exit 2 ;;\n\
                 esac\n",
                self.docker_log.display()
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
        "gpt-webai-cli-allocate-{prefix}-{}-{nonce}",
        std::process::id()
    ))
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}
