use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::cleanup::{cleanup_state, CleanupMode};
use gpt_webai_lifecycle::locks::acquire_slot_lease;
use gpt_webai_lifecycle::records::{holder_count, lock_count};

#[test]
fn cleanup_apply_removes_expired_leases_and_preserves_active_leases() {
    let root = temp_state_root("cleanup-expired");
    acquire_slot_lease(&root, "slot-01", "request-stale", "run-stale", "stale", 0)
        .expect("stale lease");
    acquire_slot_lease(
        &root,
        "slot-02",
        "request-active",
        "run-active",
        "active",
        600_000,
    )
    .expect("active lease");

    let dry_run = cleanup_state(&root, CleanupMode::DryRun);
    assert_eq!(dry_run.stale_locks, 1);
    assert_eq!(dry_run.stale_holders, 1);
    assert_eq!(lock_count(&root), 2);
    assert_eq!(holder_count(&root), 2);

    let applied = cleanup_state(&root, CleanupMode::Apply);
    assert_eq!(applied.removed_locks, 1);
    assert_eq!(applied.removed_holders, 1);
    assert_eq!(lock_count(&root), 1);
    assert_eq!(holder_count(&root), 1);
    assert!(root.join("locks/slots/slot-02.lock/lease.json").is_file());
    assert!(root.join("holders/slot-02.holder.json").is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_cli_dry_run_reports_json() {
    let root = temp_state_root("cleanup-cli");
    acquire_slot_lease(&root, "slot-01", "request-stale", "run-stale", "stale", 0)
        .expect("stale lease");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .args(["cleanup", "--json", "--dry-run"])
        .output()
        .expect("cleanup cli");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["ok"], true);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["status"], "cleanup.plan");
    assert_eq!(value["resultKind"], "cleanup.plan");
    assert!(value["message"].as_str().unwrap().contains("stale_locks=1"));
    assert_eq!(lock_count(&root), 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_cli_reports_lock_contention_without_cleaning_state() {
    let root = temp_state_root("cleanup-lock-contended");
    for path in [
        root.join("journal"),
        root.join("journal/locks"),
        root.join("journal/locks/mutation.lock"),
    ] {
        fs::create_dir(&path).expect("private contended lock component");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("private contended lock mode");
    }
    acquire_slot_lease(&root, "slot-01", "request-stale", "run-stale", "stale", 0)
        .expect("stale lease");

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .args(["cleanup", "--json", "--apply"])
        .output()
        .expect("cleanup cli");

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["resultKind"], "cleanup.lock_contended");
    assert_eq!(value["reason"], "lock.contended");
    assert_eq!(lock_count(&root), 1);
    assert_eq!(holder_count(&root), 1);

    let _ = fs::remove_dir_all(root);
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("gpt-webai-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
