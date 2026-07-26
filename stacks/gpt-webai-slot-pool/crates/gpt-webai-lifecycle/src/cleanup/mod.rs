use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::locks::LeaseRecord;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupMode {
    DryRun,
    Apply,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOutput {
    pub schema: String,
    pub ok: bool,
    pub mode: String,
    pub stale_holders: usize,
    pub stale_locks: usize,
    pub removed_holders: usize,
    pub removed_locks: usize,
    pub skipped: usize,
    pub message: String,
}

pub fn cleanup_state(state_root: &Path, mode: CleanupMode) -> CleanupOutput {
    let stale_locks = stale_locks(state_root);
    let stale_holders = stale_holders(state_root);
    let mut removed_locks = 0;
    let mut removed_holders = 0;
    let mut skipped = 0;

    if mode == CleanupMode::Apply {
        for item in &stale_locks {
            if remove_lock(state_root, item).is_ok() {
                removed_locks += 1;
            } else {
                skipped += 1;
            }
        }
        for item in &stale_holders {
            if fs::remove_file(&item.path).is_ok() || !item.path.exists() {
                removed_holders += 1;
            } else {
                skipped += 1;
            }
        }
    }

    CleanupOutput {
        schema: "gpt-webai.lifecycle.cleanup.v2".to_string(),
        ok: true,
        mode: mode_name(mode).to_string(),
        stale_holders: stale_holders.len(),
        stale_locks: stale_locks.len(),
        removed_holders,
        removed_locks,
        skipped,
        message: "cleanup inspected Rust lifecycle holders and slot locks".to_string(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StaleLock {
    slot_id: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StaleHolder {
    path: PathBuf,
}

fn stale_locks(state_root: &Path) -> Vec<StaleLock> {
    let root = state_root.join("locks").join("slots");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| stale_lock(entry.path()))
        .collect()
}

fn stale_lock(path: PathBuf) -> Option<StaleLock> {
    if !path.is_dir() {
        return None;
    }
    let record = read_lease(path.join("lease.json").as_path())?;
    if lease_is_active(&record) {
        return None;
    }
    Some(StaleLock {
        slot_id: record.slot_id,
        path,
    })
}

fn stale_holders(state_root: &Path) -> Vec<StaleHolder> {
    let root = state_root.join("holders");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| stale_holder(state_root, entry.path()))
        .collect()
}

fn stale_holder(state_root: &Path, path: PathBuf) -> Option<StaleHolder> {
    if !path.is_file() {
        return None;
    }
    let record = read_lease(&path)?;
    let lock_path = state_root
        .join("locks")
        .join("slots")
        .join(format!("{}.lock", record.slot_id))
        .join("lease.json");
    if lock_path.is_file() && lease_is_active(&record) {
        return None;
    }
    Some(StaleHolder { path })
}

fn remove_lock(state_root: &Path, lock: &StaleLock) -> std::io::Result<()> {
    let holder = state_root
        .join("holders")
        .join(format!("{}.holder.json", lock.slot_id));
    let _ = fs::remove_file(holder);
    fs::remove_dir_all(&lock.path)
}

fn read_lease(path: &Path) -> Option<LeaseRecord> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<LeaseRecord>(&text).ok()
}

fn lease_is_active(record: &LeaseRecord) -> bool {
    record.expires_at_ms > now_ms() && pid_alive(record.owner_pid)
}

fn pid_alive(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn now_ms() -> u128 {
    u128::from(crate::config::now_ms())
}

fn mode_name(mode: CleanupMode) -> &'static str {
    match mode {
        CleanupMode::DryRun => "dry-run",
        CleanupMode::Apply => "apply",
    }
}
