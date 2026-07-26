use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const LEASE_SCHEMA: &str = "gpt-webai.lease.v2";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRecord {
    pub schema: String,
    pub slot_id: String,
    pub request_id: String,
    pub run_id: String,
    pub fencing_token_sha256: String,
    pub owner_pid: u32,
    pub acquired_at_ms: u128,
    pub heartbeat_at_ms: u128,
    pub expires_at_ms: u128,
}

#[derive(Debug, Error)]
pub enum LockError {
    #[error("slot lock is busy: {0}")]
    Busy(String),
    #[error("slot lock is missing: {0}")]
    Missing(String),
    #[error("slot lock fencing token mismatch: {0}")]
    FencingMismatch(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn acquire_slot_lease(
    state_root: &Path,
    slot_id: &str,
    request_id: &str,
    run_id: &str,
    fencing_token: &str,
    ttl_ms: u128,
) -> Result<LeaseRecord, LockError> {
    let lock_dir = slot_lock_dir(state_root, slot_id);
    crate::provider_runner::create_private_directory(
        state_root,
        lock_dir.parent().expect("slot lock dir has parent"),
    )?;
    // The parent chain is descriptor-validated above; preserve atomic lock acquisition at 0700.
    match DirBuilder::new().mode(0o700).create(&lock_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(LockError::Busy(slot_id.to_string()));
        }
        Err(error) => return Err(error.into()),
    }

    let now = now_ms();
    let record = LeaseRecord {
        schema: LEASE_SCHEMA.to_string(),
        slot_id: slot_id.to_string(),
        request_id: request_id.to_string(),
        run_id: run_id.to_string(),
        fencing_token_sha256: sha256_hex(fencing_token),
        owner_pid: std::process::id(),
        acquired_at_ms: now,
        heartbeat_at_ms: now,
        expires_at_ms: now.saturating_add(ttl_ms),
    };

    if let Err(error) = write_lease_record(&lock_dir, &record)
        .and_then(|_| write_holder_record(state_root, slot_id, &record))
    {
        let _ = fs::remove_dir_all(&lock_dir);
        return Err(error.into());
    }
    Ok(record)
}

pub fn read_slot_lease(state_root: &Path, slot_id: &str) -> Result<LeaseRecord, LockError> {
    let path = lease_record_path(state_root, slot_id);
    let text = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LockError::Missing(slot_id.to_string())
        } else {
            LockError::Io(error)
        }
    })?;
    let record = serde_json::from_str::<LeaseRecord>(&text)?;
    if record.schema != LEASE_SCHEMA {
        return Err(LockError::Json(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unknown lease schema",
        ))));
    }
    Ok(record)
}

pub fn slot_lock_exists(state_root: &Path, slot_id: &str) -> bool {
    lease_record_path(state_root, slot_id).is_file()
}

pub fn release_slot_lease(
    state_root: &Path,
    slot_id: &str,
    fencing_token: &str,
) -> Result<LeaseRecord, LockError> {
    let record = read_slot_lease(state_root, slot_id)?;
    if record.fencing_token_sha256 != sha256_hex(fencing_token) {
        return Err(LockError::FencingMismatch(slot_id.to_string()));
    }
    remove_slot_lease_files(state_root, slot_id)?;
    Ok(record)
}

pub fn release_stale_slot_lease(
    state_root: &Path,
    slot_id: &str,
) -> Result<LeaseRecord, LockError> {
    let record = read_slot_lease(state_root, slot_id)?;
    if !lease_is_stale(&record) {
        return Err(LockError::Busy(slot_id.to_string()));
    }
    remove_slot_lease_files(state_root, slot_id)?;
    Ok(record)
}

pub fn lease_is_stale(record: &LeaseRecord) -> bool {
    record.expires_at_ms <= now_ms() || !pid_alive(record.owner_pid)
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn remove_slot_lease_files(state_root: &Path, slot_id: &str) -> std::io::Result<()> {
    let _ = fs::remove_file(slot_holder_path(state_root, slot_id));
    fs::remove_dir_all(slot_lock_dir(state_root, slot_id))
}

fn pid_alive(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn write_lease_record(lock_dir: &Path, record: &LeaseRecord) -> std::io::Result<()> {
    fs::write(
        lock_dir.join("lease.json"),
        format!("{}\n", serde_json::to_string_pretty(record)?),
    )
}

fn write_holder_record(
    state_root: &Path,
    slot_id: &str,
    record: &LeaseRecord,
) -> std::io::Result<()> {
    let path = slot_holder_path(state_root, slot_id);
    crate::provider_runner::create_private_directory(
        state_root,
        path.parent().expect("slot holder path has parent"),
    )?;
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(record)?))
}

fn slot_lock_dir(state_root: &Path, slot_id: &str) -> PathBuf {
    state_root
        .join("locks")
        .join("slots")
        .join(format!("{slot_id}.lock"))
}

fn lease_record_path(state_root: &Path, slot_id: &str) -> PathBuf {
    slot_lock_dir(state_root, slot_id).join("lease.json")
}

fn slot_holder_path(state_root: &Path, slot_id: &str) -> PathBuf {
    state_root
        .join("holders")
        .join(format!("{slot_id}.holder.json"))
}

fn now_ms() -> u128 {
    u128::from(crate::config::now_ms())
}
