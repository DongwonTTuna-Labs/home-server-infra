use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::ids::{validate_event_id, validate_h256, validate_timestamp_ms};
use crate::journal::canonical::{canonical_bytes, parse_canonical};

pub const HEAD_SCHEMA: &str = "pr72.head.r13.v1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Head {
    pub head_generation: u64,
    pub last_event_id: Option<String>,
    pub projection_digest: String,
    pub schema_version: String,
    pub snapshot_event_id: Option<String>,
    pub snapshot_sha256: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct HeadStore {
    state_root: PathBuf,
}

#[derive(Debug, Error)]
pub enum HeadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("journal.head_cas_conflict")]
    CasConflict,
    #[error("journal lock contended: {0}")]
    LockContended(&'static str),
    #[error("HEAD contract invalid: {0}")]
    Invalid(&'static str),
}

pub struct MutationGuard {
    head: DirectoryLock,
    mutation: DirectoryLock,
    state_root: PathBuf,
}

struct DirectoryLock {
    path: PathBuf,
}

impl HeadStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn acquire_mutation(&self) -> Result<MutationGuard, HeadError> {
        let locks = self.state_root.join("journal/locks");
        crate::provider_runner::create_private_directory(&self.state_root, &locks)?;
        let mutation = DirectoryLock::acquire(locks.join("mutation.lock"), "mutation")?;
        let head = DirectoryLock::acquire(locks.join("head.lock"), "head")?;
        Ok(MutationGuard {
            head,
            mutation,
            state_root: self.state_root.clone(),
        })
    }

    pub fn read(&self) -> Result<Option<Head>, HeadError> {
        let path = self.state_root.join("journal/HEAD.json");
        if !path.exists() {
            return Ok(None);
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file()
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(HeadError::Invalid("unsafe HEAD file"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        parse_canonical(&bytes)?;
        let head: Head = serde_json::from_slice(&bytes)?;
        head.validate()?;
        Ok(Some(head))
    }

    pub fn publish(
        &self,
        _guard: &MutationGuard,
        expected_generation: u64,
        head: &Head,
    ) -> Result<(), HeadError> {
        let current = self.read()?.map_or(0, |value| value.head_generation);
        if current != expected_generation || head.head_generation != current + 1 {
            return Err(HeadError::CasConflict);
        }
        head.validate()?;
        let directory = self.state_root.join("journal");
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let target = directory.join("HEAD.json");
        let temp = directory.join(format!(
            ".HEAD.json.{}.{}.tmp",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = canonical_bytes(head)?;
        write_temp(&temp, &bytes)?;
        fs::rename(&temp, &target)?;
        File::open(&directory)?.sync_all()?;
        let reopened = read_safe(&target)?;
        if reopened != bytes {
            return Err(HeadError::CasConflict);
        }
        Ok(())
    }

    pub fn publish_with_retry(
        &self,
        guard: &MutationGuard,
        mut expected_generation: u64,
        head: &Head,
    ) -> Result<Head, HeadError> {
        let mut candidate = head.clone();
        for conflict_index in 0..=3 {
            candidate.head_generation = expected_generation
                .checked_add(1)
                .ok_or(HeadError::Invalid("headGeneration overflow"))?;
            match self.publish(guard, expected_generation, &candidate) {
                Ok(()) => return Ok(candidate),
                Err(HeadError::CasConflict) if conflict_index < 3 => {
                    expected_generation = self.read()?.map_or(0, |current| current.head_generation);
                }
                Err(error) => return Err(error),
            }
        }
        Err(HeadError::CasConflict)
    }

    pub fn replace_after_replay(
        &self,
        _guard: &MutationGuard,
        head: &Head,
    ) -> Result<Head, HeadError> {
        let mut candidate = head.clone();
        candidate.head_generation = 1;
        candidate.validate()?;
        self.replace(&candidate)?;
        Ok(candidate)
    }

    fn replace(&self, head: &Head) -> Result<(), HeadError> {
        let directory = self.state_root.join("journal");
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let target = directory.join("HEAD.json");
        let temp = directory.join(format!(
            ".HEAD.json.replay.{}.{}.tmp",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = canonical_bytes(head)?;
        write_temp(&temp, &bytes)?;
        fs::rename(&temp, &target)?;
        File::open(&directory)?.sync_all()?;
        let reopened = read_safe(&target)?;
        if reopened != bytes {
            return Err(HeadError::Invalid("HEAD reopen mismatch"));
        }
        Ok(())
    }
}

impl Head {
    pub fn validate(&self) -> Result<(), HeadError> {
        let pairwise_snapshot_null =
            self.snapshot_event_id.is_none() == self.snapshot_sha256.is_none();
        let valid = self.schema_version == HEAD_SCHEMA
            && self.head_generation > 0
            && validate_h256(&self.projection_digest).is_ok()
            && validate_timestamp_ms(self.updated_at_ms).is_ok()
            && self
                .last_event_id
                .as_deref()
                .is_none_or(|value| validate_event_id(value).is_ok())
            && self
                .snapshot_event_id
                .as_deref()
                .is_none_or(|value| validate_event_id(value).is_ok())
            && self
                .snapshot_sha256
                .as_deref()
                .is_none_or(|value| validate_h256(value).is_ok())
            && pairwise_snapshot_null;
        valid.then_some(()).ok_or(HeadError::Invalid("fields"))
    }
}

impl DirectoryLock {
    fn acquire(path: PathBuf, name: &'static str) -> Result<Self, HeadError> {
        // HeadStore validates the complete parent chain before this atomic lock-leaf create.
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => Ok(Self { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(HeadError::LockContended(name))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let _ = (&self.head, &self.mutation);
    }
}

impl MutationGuard {
    pub(crate) fn authorizes(&self, state_root: &Path) -> bool {
        self.state_root == state_root
    }
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn read_safe(path: &Path) -> Result<Vec<u8>, HeadError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(HeadError::Invalid("unsafe HEAD file"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
