use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::contracts::events::{EventEnvelope, EventError};
use crate::contracts::ids::validate_event_id;
use crate::journal::canonical::{canonical_bytes, parse_canonical};
use crate::journal::head::{Head, HeadError, HeadStore, MutationGuard, HEAD_SCHEMA};
use crate::journal::projection::{
    reduce, PersistedSessionSeed, ProjectionError, ProjectionStore, ReducedProjection,
};
use crate::journal::replay::{topological, ReplayError};
use crate::journal::snapshot::{SnapshotInspection, SnapshotStore};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct EventStore {
    state_root: PathBuf,
}

#[derive(Debug, Error)]
pub enum EventStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("event invalid: {0}")]
    Event(#[from] EventError),
    #[error("json invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("journal.immutable_collision: {0}")]
    ImmutableCollision(String),
    #[error("unsafe journal file: {0}")]
    UnsafeFile(PathBuf),
    #[error("journal replay invalid: {0}")]
    Replay(#[from] ReplayError),
    #[error("HEAD invalid: {0}")]
    Head(#[from] HeadError),
    #[error("projection invalid: {0}")]
    Projection(#[from] ProjectionError),
    #[error("mutation guard belongs to another state root")]
    WrongMutationGuard,
}

#[derive(Clone, Debug)]
pub struct CommitResult {
    pub event_paths: Vec<PathBuf>,
    pub head: Head,
    pub projection: ReducedProjection,
}

#[derive(Clone, Debug)]
pub struct DerivedInspection {
    pub projection: ReducedProjection,
    pub head_matches: bool,
    pub projections_match: bool,
    pub trusted_snapshot: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RebuildHeadObservation {
    Match,
    Stale(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebuildInspection {
    pub head: RebuildHeadObservation,
    pub snapshot: SnapshotInspection,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RebuildCheckError {
    #[error("event invalid: {0}")]
    EventInvalid(String),
    #[error("transition invalid: {0}")]
    TransitionInvalid(String),
    #[error("projection digest mismatch: {0}")]
    DigestMismatch(String),
}

impl EventStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn events_dir(&self) -> PathBuf {
        self.state_root.join("journal/events")
    }

    pub fn append(
        &self,
        guard: &MutationGuard,
        event: &EventEnvelope,
    ) -> Result<PathBuf, EventStoreError> {
        self.append_transaction(guard, std::slice::from_ref(event))?
            .into_iter()
            .next()
            .ok_or_else(|| EventStoreError::ImmutableCollision(event.event_id.clone()))
    }

    pub fn append_transaction(
        &self,
        guard: &MutationGuard,
        transaction: &[EventEnvelope],
    ) -> Result<Vec<PathBuf>, EventStoreError> {
        Ok(self
            .append_transaction_with_seeds(guard, transaction, &BTreeMap::new())?
            .event_paths)
    }

    pub fn append_transaction_with_seeds(
        &self,
        guard: &MutationGuard,
        transaction: &[EventEnvelope],
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Result<CommitResult, EventStoreError> {
        if !guard.authorizes(&self.state_root) {
            return Err(EventStoreError::WrongMutationGuard);
        }
        if transaction.is_empty() {
            return Err(EventStoreError::Replay(ReplayError::Invalid(
                "empty transaction".to_string(),
            )));
        }
        let existing = self.load_all()?;
        let mut combined = existing.clone();
        let mut new_ids = std::collections::BTreeSet::new();
        for event in transaction {
            event.validate()?;
            match existing
                .iter()
                .find(|known| known.event_id == event.event_id)
            {
                Some(known) if known == event => continue,
                Some(_) => {
                    return Err(EventStoreError::ImmutableCollision(event.event_id.clone()));
                }
                None => {
                    new_ids.insert(event.event_id.clone());
                    combined.push(event.clone());
                }
            }
        }
        let ordered = topological(&combined)?;
        let projection = reduce(&ordered, seeds)?;
        let mut paths = Vec::new();
        for event in ordered
            .iter()
            .filter(|event| new_ids.contains(&event.event_id))
        {
            paths.push(self.append_one(event)?);
        }
        crate::failpoint::hit("after-event-append-before-head");
        if new_ids.is_empty() {
            paths = transaction
                .iter()
                .map(|event| self.events_dir().join(format!("{}.json", event.event_id)))
                .collect();
            let head_store = HeadStore::new(&self.state_root);
            if let Ok(Some(head)) = head_store.read() {
                let projections_match = ProjectionStore::new(&self.state_root)
                    .read_all()
                    .is_ok_and(|files| files == projection.files);
                if projections_match
                    && head.last_event_id == projection.state.last_event_id
                    && head.projection_digest == projection.state.projection_digest
                {
                    return Ok(CommitResult {
                        event_paths: paths,
                        head,
                        projection,
                    });
                }
            }
        }
        let head_store = HeadStore::new(&self.state_root);
        let current_head = head_store.read();
        let (expected_generation, snapshot_event_id, snapshot_sha256, recovered) =
            match current_head {
                Ok(Some(head)) => {
                    let trusted = SnapshotStore::new(&self.state_root)
                        .load_trusted(&head, &existing, seeds)
                        .is_some();
                    (
                        head.head_generation,
                        trusted.then_some(head.snapshot_event_id).flatten(),
                        trusted.then_some(head.snapshot_sha256).flatten(),
                        false,
                    )
                }
                Ok(None) | Err(_) => (0, None, None, true),
            };
        let updated_at_ms = ordered
            .last()
            .map(|event| event.created_at_ms)
            .ok_or_else(|| EventStoreError::Replay(ReplayError::Invalid("empty journal".into())))?;
        let candidate = Head {
            head_generation: expected_generation.saturating_add(1),
            last_event_id: projection.state.last_event_id.clone(),
            projection_digest: projection.state.projection_digest.clone(),
            schema_version: HEAD_SCHEMA.to_string(),
            snapshot_event_id,
            snapshot_sha256,
            updated_at_ms,
        };
        let head = if recovered {
            head_store.replace_after_replay(guard, &candidate)?
        } else {
            head_store.publish_with_retry(guard, expected_generation, &candidate)?
        };
        crate::failpoint::hit("after-head-before-projection-publish");
        let operation_id = transaction
            .first()
            .map(|event| event.operation_id.as_str())
            .expect("nonempty transaction");
        ProjectionStore::new(&self.state_root).publish(guard, operation_id, &projection)?;
        let reopened = head_store
            .read()?
            .ok_or(HeadError::Invalid("HEAD missing after publish"))?;
        if reopened != head {
            return Err(EventStoreError::Head(HeadError::Invalid(
                "HEAD changed after projection publish",
            )));
        }
        Ok(CommitResult {
            event_paths: paths,
            head,
            projection,
        })
    }

    pub fn replay(
        &self,
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Result<ReducedProjection, EventStoreError> {
        Ok(reduce(&self.load_all()?, seeds)?)
    }

    pub fn inspect_derived(
        &self,
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Result<DerivedInspection, EventStoreError> {
        let events = self.load_all()?;
        let projection = reduce(&events, seeds)?;
        let head = HeadStore::new(&self.state_root).read();
        let trusted_snapshot = head
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .is_some_and(|head| {
                SnapshotStore::new(&self.state_root)
                    .load_trusted(head, &events, seeds)
                    .is_some()
            });
        let head_matches = head
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .is_some_and(|head| {
                head.last_event_id == projection.state.last_event_id
                    && head.projection_digest == projection.state.projection_digest
                    && head.updated_at_ms >= projection.state.last_event_created_at_ms
            });
        let projections_match = ProjectionStore::new(&self.state_root)
            .read_all()
            .is_ok_and(|files| files == projection.files);
        Ok(DerivedInspection {
            projection,
            head_matches,
            projections_match,
            trusted_snapshot,
        })
    }

    pub fn inspect_rebuild_check_only(
        &self,
        seeds: &BTreeMap<String, PersistedSessionSeed>,
    ) -> Result<RebuildInspection, RebuildCheckError> {
        let events = self
            .load_all()
            .map_err(|error| RebuildCheckError::EventInvalid(error.to_string()))?;
        let projection = reduce(&events, seeds).map_err(classify_projection_replay_error)?;
        let persisted = ProjectionStore::new(&self.state_root)
            .read_all()
            .map_err(|error| RebuildCheckError::DigestMismatch(error.to_string()))?;
        if persisted != projection.files {
            return Err(RebuildCheckError::DigestMismatch(
                "persisted projections differ from replay".to_string(),
            ));
        }

        let head_result = HeadStore::new(&self.state_root).read();
        let head = match &head_result {
            Ok(Some(head)) => Some(head),
            Ok(None) | Err(_) => None,
        };
        let head_observation = match &head_result {
            Ok(Some(head))
                if head.last_event_id == projection.state.last_event_id
                    && head.projection_digest == projection.state.projection_digest
                    && head.updated_at_ms >= projection.state.last_event_created_at_ms =>
            {
                RebuildHeadObservation::Match
            }
            Ok(Some(_)) => {
                RebuildHeadObservation::Stale("HEAD does not match replayed projection".to_string())
            }
            Ok(None) => RebuildHeadObservation::Stale("HEAD is missing".to_string()),
            Err(error) => RebuildHeadObservation::Stale(error.to_string()),
        };
        let snapshot = head.map_or(SnapshotInspection::Absent, |head| {
            SnapshotStore::new(&self.state_root).inspect_head_snapshot(head, &events, seeds)
        });
        Ok(RebuildInspection {
            head: head_observation,
            snapshot,
        })
    }

    pub fn rebuild_derived(
        &self,
        guard: &MutationGuard,
        seeds: &BTreeMap<String, PersistedSessionSeed>,
        operation_id: &str,
        now_ms: u64,
    ) -> Result<CommitResult, EventStoreError> {
        if !guard.authorizes(&self.state_root) {
            return Err(EventStoreError::WrongMutationGuard);
        }
        let events = self.load_all()?;
        let projection = reduce(&events, seeds)?;
        let head_store = HeadStore::new(&self.state_root);
        let current = head_store.read();
        let (expected_generation, snapshot_event_id, snapshot_sha256, recovered) = match current {
            Ok(Some(head)) => {
                let trusted = SnapshotStore::new(&self.state_root)
                    .load_trusted(&head, &events, seeds)
                    .is_some();
                (
                    head.head_generation,
                    trusted.then_some(head.snapshot_event_id).flatten(),
                    trusted.then_some(head.snapshot_sha256).flatten(),
                    false,
                )
            }
            Ok(None) | Err(_) => (0, None, None, true),
        };
        let candidate = Head {
            head_generation: expected_generation.saturating_add(1),
            last_event_id: projection.state.last_event_id.clone(),
            projection_digest: projection.state.projection_digest.clone(),
            schema_version: HEAD_SCHEMA.to_string(),
            snapshot_event_id,
            snapshot_sha256,
            updated_at_ms: now_ms.max(projection.state.last_event_created_at_ms),
        };
        let head = if recovered {
            head_store.replace_after_replay(guard, &candidate)?
        } else {
            head_store.publish_with_retry(guard, expected_generation, &candidate)?
        };
        ProjectionStore::new(&self.state_root).publish(guard, operation_id, &projection)?;
        Ok(CommitResult {
            event_paths: Vec::new(),
            head,
            projection,
        })
    }

    fn append_one(&self, event: &EventEnvelope) -> Result<PathBuf, EventStoreError> {
        event.validate()?;
        let bytes = canonical_bytes(event)?;
        let directory = self.events_dir();
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let target = directory.join(format!("{}.json", event.event_id));
        let temp = directory.join(format!(
            ".{}.{}.{}.{}.tmp",
            event.event_id,
            event.operation_id,
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        write_temp(&temp, &bytes)?;
        crate::failpoint::hit("after-immutable-temp-write");
        match rename_noreplace(&temp, &target) {
            Ok(()) => {
                crate::failpoint::hit("after-immutable-promote-before-directory-fsync");
                sync_dir(&directory)?;
                verify_file(&target, &bytes)?;
                Ok(target)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let result = verify_file(&target, &bytes);
                let _ = fs::remove_file(&temp);
                match result {
                    Ok(()) => Ok(target),
                    Err(_) => {
                        self.quarantine(event, &bytes)?;
                        Err(EventStoreError::ImmutableCollision(event.event_id.clone()))
                    }
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                Err(error.into())
            }
        }
    }

    pub fn load_all(&self) -> Result<Vec<EventEnvelope>, EventStoreError> {
        let directory = self.events_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                return Err(EventStoreError::UnsafeFile(path));
            };
            if name.starts_with('.') && name.ends_with(".tmp") {
                continue;
            }
            let valid = path
                .extension()
                .is_some_and(|extension| extension == "json")
                && path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| validate_event_id(value).is_ok());
            if !valid {
                return Err(EventStoreError::UnsafeFile(path));
            }
            paths.push(path);
        }
        paths.sort();
        paths.into_iter().map(read_event).collect()
    }

    fn quarantine(&self, event: &EventEnvelope, bytes: &[u8]) -> Result<(), EventStoreError> {
        let directory = self.state_root.join("journal/quarantine");
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let path = directory.join(format!("{}.{}.json", event.event_id, event.operation_id));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                sync_dir(&directory)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn classify_projection_replay_error(error: ProjectionError) -> RebuildCheckError {
    match error {
        ProjectionError::Replay(
            error @ (ReplayError::Invalid(_)
            | ReplayError::Duplicate(_)
            | ReplayError::Missing(_)
            | ReplayError::Cycle),
        ) => RebuildCheckError::EventInvalid(error.to_string()),
        ProjectionError::Replay(error) => RebuildCheckError::TransitionInvalid(error.to_string()),
        error => RebuildCheckError::TransitionInvalid(error.to_string()),
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

fn verify_file(path: &Path, expected: &[u8]) -> Result<(), EventStoreError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(EventStoreError::UnsafeFile(path.to_path_buf()));
    }
    let mut actual = Vec::new();
    file.read_to_end(&mut actual)?;
    parse_canonical(&actual)?;
    if actual != expected {
        return Err(EventStoreError::ImmutableCollision(
            path.display().to_string(),
        ));
    }
    Ok(())
}

fn read_event(path: PathBuf) -> Result<EventEnvelope, EventStoreError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(EventStoreError::UnsafeFile(path));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    parse_canonical(&bytes)?;
    let event: EventEnvelope = serde_json::from_slice(&bytes)?;
    event.validate()?;
    if path.file_stem().and_then(|name| name.to_str()) != Some(event.event_id.as_str()) {
        return Err(EventStoreError::UnsafeFile(path));
    }
    Ok(event)
}

fn sync_dir(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

fn rename_noreplace(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in source path"))?;
    let target = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in target path"))?;
    // SAFETY: both C strings remain alive for the syscall and contain no interior NUL.
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
