use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::events::EventEnvelope;
use crate::contracts::ids::{h256, validate_event_id, validate_h256, validate_timestamp_ms};
use crate::contracts::projection::ProjectionState;
use crate::journal::canonical::{canonical_bytes, parse_canonical};
use crate::journal::head::{Head, MutationGuard};
use crate::journal::projection::{reduce, PersistedSessionSeed, ProjectionError};
use crate::journal::replay::{topological, ReplayError};

pub const SNAPSHOT_SCHEMA: &str = "pr72.snapshot.r13.v1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: String,
    pub last_event_id: String,
    pub last_event_created_at_ms: u64,
    pub projection: ProjectionState,
    pub projection_digest: String,
    pub previous_snapshot_digest: Option<String>,
    pub snapshot_created_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct SnapshotStore {
    state_root: PathBuf,
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("snapshot I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot replay error: {0}")]
    Replay(#[from] ReplayError),
    #[error("snapshot projection error: {0}")]
    Projection(#[from] ProjectionError),
    #[error("snapshot invalid: {0}")]
    Invalid(&'static str),
    #[error("snapshot immutable collision: {0}")]
    ImmutableCollision(PathBuf),
    #[error("unsafe snapshot path: {0}")]
    UnsafePath(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotInspection {
    Absent,
    Trusted,
    Ignored(String),
}

impl Snapshot {
    pub fn new(
        projection: ProjectionState,
        previous_snapshot_digest: Option<String>,
        snapshot_created_at_ms: u64,
    ) -> Result<Self, SnapshotError> {
        let last_event_id = projection
            .last_event_id
            .clone()
            .ok_or(SnapshotError::Invalid("empty projection"))?;
        let snapshot = Self {
            schema_version: SNAPSHOT_SCHEMA.to_string(),
            last_event_id,
            last_event_created_at_ms: projection.last_event_created_at_ms,
            projection_digest: projection.projection_digest.clone(),
            projection,
            previous_snapshot_digest,
            snapshot_created_at_ms,
        };
        snapshot.validate_shape()?;
        Ok(snapshot)
    }

    pub fn validate_shape(&self) -> Result<(), SnapshotError> {
        if self.schema_version != SNAPSHOT_SCHEMA
            || validate_event_id(&self.last_event_id).is_err()
            || validate_timestamp_ms(self.last_event_created_at_ms).is_err()
            || validate_timestamp_ms(self.snapshot_created_at_ms).is_err()
            || self.snapshot_created_at_ms < self.last_event_created_at_ms
            || validate_h256(&self.projection_digest).is_err()
            || self
                .previous_snapshot_digest
                .as_deref()
                .is_some_and(|digest| validate_h256(digest).is_err())
            || self.projection.last_event_id.as_deref() != Some(self.last_event_id.as_str())
            || self.projection.last_event_created_at_ms != self.last_event_created_at_ms
            || self.projection.projection_digest != self.projection_digest
        {
            return Err(SnapshotError::Invalid("fields"));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, SnapshotError> {
        Ok(h256(canonical_bytes(self)?))
    }
}

impl SnapshotStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn directory(&self) -> PathBuf {
        self.state_root.join("journal/snapshots")
    }

    pub fn publish(
        &self,
        guard: &MutationGuard,
        operation_id: &str,
        snapshot: &Snapshot,
    ) -> Result<(PathBuf, String), SnapshotError> {
        if !guard.authorizes(&self.state_root) {
            return Err(SnapshotError::Invalid("wrong mutation guard"));
        }
        snapshot.validate_shape()?;
        let directory = self.directory();
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let target = directory.join(format!("{}.json", snapshot.last_event_id));
        let temp = directory.join(format!(
            ".{}.{}.{}.{}.tmp",
            snapshot.last_event_id,
            operation_id,
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = canonical_bytes(snapshot)?;
        write_private(&temp, &bytes)?;
        match rename_noreplace(&temp, &target) {
            Ok(()) => File::open(&directory)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = read_private(&target)?;
                let _ = fs::remove_file(&temp);
                if existing != bytes {
                    return Err(SnapshotError::ImmutableCollision(target));
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        let reopened = read_private(&target)?;
        if reopened != bytes {
            return Err(SnapshotError::ImmutableCollision(target));
        }
        Ok((target, h256(bytes)))
    }

    pub fn load_trusted(
        &self,
        head: &Head,
        events: &[EventEnvelope],
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Option<Snapshot> {
        self.verify_head_snapshot(head, events, seeds)
            .ok()
            .flatten()
    }

    pub fn inspect_head_snapshot(
        &self,
        head: &Head,
        events: &[EventEnvelope],
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> SnapshotInspection {
        if head.snapshot_event_id.is_none() && head.snapshot_sha256.is_none() {
            return SnapshotInspection::Absent;
        }
        match self.verify_head_snapshot(head, events, seeds) {
            Ok(Some(_)) => SnapshotInspection::Trusted,
            Ok(None) => SnapshotInspection::Absent,
            Err(error) => SnapshotInspection::Ignored(error.to_string()),
        }
    }

    fn verify_head_snapshot(
        &self,
        head: &Head,
        events: &[EventEnvelope],
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Result<Option<Snapshot>, SnapshotError> {
        let (Some(event_id), Some(expected_sha)) = (
            head.snapshot_event_id.as_deref(),
            head.snapshot_sha256.as_deref(),
        ) else {
            return Ok(None);
        };
        let candidates = self.read_candidates()?;
        let mut by_digest = BTreeMap::new();
        for (path, bytes, snapshot) in &candidates {
            by_digest.insert(h256(bytes), (path, snapshot));
        }
        let (_, snapshot) = by_digest
            .get(expected_sha)
            .ok_or(SnapshotError::Invalid("snapshotSha256"))?;
        if snapshot.last_event_id != event_id {
            return Err(SnapshotError::Invalid("snapshotEventId"));
        }
        let mut visited = BTreeSet::new();
        self.verify_chain(snapshot, &by_digest, events, seeds, &mut visited)?;
        Ok(Some((*snapshot).clone()))
    }

    fn verify_chain(
        &self,
        snapshot: &Snapshot,
        by_digest: &BTreeMap<String, (&PathBuf, &Snapshot)>,
        events: &[EventEnvelope],
        seeds: &BTreeMap<String, PersistedSessionSeed>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), SnapshotError> {
        snapshot.validate_shape()?;
        let digest = snapshot.sha256()?;
        if !visited.insert(digest) {
            return Err(SnapshotError::Invalid("snapshot cycle"));
        }
        let ordered = topological(events)?;
        let position = ordered
            .iter()
            .position(|event| event.event_id == snapshot.last_event_id)
            .ok_or(SnapshotError::Invalid("snapshot event missing"))?;
        if let Some(previous) = &snapshot.previous_snapshot_digest {
            let (_, prior) = by_digest
                .get(previous)
                .ok_or(SnapshotError::Invalid("previousSnapshotDigest"))?;
            if prior.snapshot_created_at_ms > snapshot.snapshot_created_at_ms {
                return Err(SnapshotError::Invalid("snapshot chronology"));
            }
            let prior_position = ordered
                .iter()
                .position(|event| event.event_id == prior.last_event_id)
                .ok_or(SnapshotError::Invalid("prior snapshot event missing"))?;
            if prior_position >= position {
                return Err(SnapshotError::Invalid("snapshot event chronology"));
            }
            self.verify_chain(prior, by_digest, events, seeds, visited)?;
        }
        let replayed = reduce(&ordered[..=position], seeds)?;
        if replayed.state != snapshot.projection
            || replayed.state.projection_digest != snapshot.projection_digest
            || replayed.state.last_event_created_at_ms != snapshot.last_event_created_at_ms
        {
            return Err(SnapshotError::Invalid("snapshot replay digest"));
        }
        Ok(())
    }

    fn read_candidates(&self) -> Result<Vec<(PathBuf, Vec<u8>, Snapshot)>, SnapshotError> {
        let directory = self.directory();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                return Err(SnapshotError::UnsafePath(path));
            };
            if name.starts_with('.') && name.ends_with(".tmp") {
                continue;
            }
            if path.extension().is_none_or(|value| value != "json")
                || path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_none_or(|value| validate_event_id(value).is_err())
            {
                return Err(SnapshotError::UnsafePath(path));
            }
            let bytes = read_private(&path)?;
            parse_canonical(&bytes)?;
            let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
            if path.file_stem().and_then(|value| value.to_str())
                != Some(snapshot.last_event_id.as_str())
            {
                return Err(SnapshotError::UnsafePath(path));
            }
            result.push((path, bytes, snapshot));
        }
        Ok(result)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_private(path: &Path) -> Result<Vec<u8>, SnapshotError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(SnapshotError::UnsafePath(path.to_path_buf()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn rename_noreplace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::ffi::OsStrExt;
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in snapshot source"))?;
    let target = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in snapshot target"))?;
    // SAFETY: the two C strings outlive the syscall and contain no interior NUL.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
