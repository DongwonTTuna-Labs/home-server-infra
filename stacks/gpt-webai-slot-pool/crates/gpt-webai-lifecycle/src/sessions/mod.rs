use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::ids::{
    validate_cohort, validate_conversation_url, validate_generation, validate_request_id,
    validate_run_id, validate_session_id, validate_slot_id, validate_timestamp_ms,
};
use crate::journal::canonical::canonical_bytes;

const SESSION_SCHEMA: &str = "pr72.persisted-session.r13.v1";
static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionRecord {
    pub schema_version: String,
    pub session_id: String,
    pub conversation_url: String,
    pub slot_id: String,
    pub cohort: String,
    pub page_binding_generation: u16,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewSessionRecord {
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: String,
    pub conversation_url: String,
    pub slot_id: String,
    pub cohort: String,
    pub page_binding_generation: u16,
}

impl SessionRecord {
    pub fn request_binding(&self) -> Result<(&str, &str), SessionRecordError> {
        match (self.request_id.as_deref(), self.run_id.as_deref()) {
            (Some(request_id), Some(run_id)) => Ok((request_id, run_id)),
            _ => Err(SessionRecordError::Invalid(
                "request binding missing".to_string(),
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum SessionRecordError {
    #[error("session record is missing: {0}")]
    Missing(String),
    #[error("session record already exists: {0}")]
    Collision(String),
    #[error("session record is invalid: {0}")]
    Invalid(String),
    #[error("session record has invalid session url: {0}")]
    InvalidConversationUrl(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn new_session_record(input: NewSessionRecord) -> Result<SessionRecord, SessionRecordError> {
    let now = now_ms()?;
    let record = SessionRecord {
        schema_version: SESSION_SCHEMA.to_string(),
        session_id: input.session_id,
        conversation_url: input.conversation_url,
        slot_id: input.slot_id,
        cohort: input.cohort,
        page_binding_generation: input.page_binding_generation,
        request_id: input.request_id,
        run_id: input.run_id,
        created_at_ms: now,
        updated_at_ms: now,
    };
    validate_session_record(&record)?;
    Ok(record)
}

pub fn write_session_record(
    state_root: &Path,
    record: &SessionRecord,
) -> Result<(), SessionRecordError> {
    validate_session_record(record)?;
    let path = session_record_path(state_root, &record.session_id)?;
    let parent = path.parent().expect("session path has parent");
    ensure_private_directory(state_root, parent)?;
    let bytes =
        canonical_bytes(record).map_err(|error| SessionRecordError::Invalid(error.to_string()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                SessionRecordError::Collision(record.session_id.clone())
            } else {
                SessionRecordError::Io(error)
            }
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    verify_session_file(&path, &bytes)?;
    Ok(())
}

pub fn update_session_record(
    state_root: &Path,
    record: &SessionRecord,
) -> Result<(), SessionRecordError> {
    validate_session_record(record)?;
    let current = read_session_record(state_root, &record.session_id)?;
    if current.session_id != record.session_id
        || current.conversation_url != record.conversation_url
        || current.slot_id != record.slot_id
        || current.cohort != record.cohort
        || current.request_id != record.request_id
        || current.run_id != record.run_id
        || current.created_at_ms != record.created_at_ms
        || record.updated_at_ms < current.updated_at_ms
    {
        return Err(SessionRecordError::Invalid(
            "session update identity".to_string(),
        ));
    }
    let path = session_record_path(state_root, &record.session_id)?;
    let parent = path.parent().expect("session path has parent");
    ensure_private_directory(state_root, parent)?;
    let temp = temp_path(&path);
    let bytes =
        canonical_bytes(record).map_err(|error| SessionRecordError::Invalid(error.to_string()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    if let Err(error) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    File::open(parent)?.sync_all()?;
    verify_session_file(&path, &bytes)?;
    Ok(())
}

pub fn read_session_record(
    state_root: &Path,
    session_id: &str,
) -> Result<SessionRecord, SessionRecordError> {
    let path = session_record_path(state_root, session_id)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SessionRecordError::Missing(session_id.to_string())
            } else {
                SessionRecordError::Io(error)
            }
        })?;
    validate_session_file_metadata(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let record = serde_json::from_slice::<SessionRecord>(&bytes)?;
    validate_session_record(&record)?;
    if record.session_id != session_id {
        return Err(SessionRecordError::Invalid(
            "session id mismatch".to_string(),
        ));
    }
    let canonical =
        canonical_bytes(&record).map_err(|error| SessionRecordError::Invalid(error.to_string()))?;
    if canonical != bytes {
        return Err(SessionRecordError::Invalid(
            "non-canonical session bytes".to_string(),
        ));
    }
    Ok(record)
}

pub fn read_request_session_record(
    state_root: &Path,
    request_id: &str,
) -> Result<SessionRecord, SessionRecordError> {
    validate_request_id(request_id)
        .map_err(|_| SessionRecordError::Invalid("request id".to_string()))?;
    let directory = state_root.join("sessions");
    let entries = fs::read_dir(&directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SessionRecordError::Missing(request_id.to_string())
        } else {
            SessionRecordError::Io(error)
        }
    })?;
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(session_id) = name.strip_suffix(".json") else {
            continue;
        };
        let record = read_session_record(state_root, session_id)?;
        if record.request_id.as_deref() == Some(request_id) {
            matches.push(record);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(SessionRecordError::Missing(request_id.to_string())),
        _ => Err(SessionRecordError::Invalid(
            "duplicate request binding".to_string(),
        )),
    }
}

pub fn read_all_session_records(
    state_root: &Path,
) -> Result<Vec<SessionRecord>, SessionRecordError> {
    let directory = state_root.join("sessions");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut session_ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| SessionRecordError::Invalid("session filename".to_string()))?;
        if name.starts_with('.') && name.ends_with(".tmp") {
            continue;
        }
        let session_id = name
            .strip_suffix(".json")
            .ok_or_else(|| SessionRecordError::Invalid("session filename".to_string()))?;
        validate_session_id(session_id)
            .map_err(|_| SessionRecordError::Invalid("session filename".to_string()))?;
        session_ids.push(session_id.to_string());
    }
    session_ids.sort();
    session_ids
        .into_iter()
        .map(|session_id| read_session_record(state_root, &session_id))
        .collect()
}

pub fn mark_session_released(mut record: SessionRecord, _reason: Option<String>) -> SessionRecord {
    record.updated_at_ms = now_ms().unwrap_or(record.updated_at_ms);
    record
}

fn validate_session_record(record: &SessionRecord) -> Result<(), SessionRecordError> {
    if record.schema_version != SESSION_SCHEMA
        || validate_session_id(&record.session_id).is_err()
        || validate_slot_id(&record.slot_id).is_err()
        || validate_cohort(&record.cohort).is_err()
        || validate_generation(record.page_binding_generation).is_err()
        || validate_timestamp_ms(record.created_at_ms).is_err()
        || validate_timestamp_ms(record.updated_at_ms).is_err()
        || record.updated_at_ms < record.created_at_ms
        || record
            .request_id
            .as_deref()
            .is_some_and(|value| validate_request_id(value).is_err())
        || record
            .run_id
            .as_deref()
            .is_some_and(|value| validate_run_id(value).is_err())
    {
        return Err(SessionRecordError::Invalid("closed schema".to_string()));
    }
    validate_conversation_url(&record.conversation_url, &record.session_id)
        .map_err(|_| SessionRecordError::InvalidConversationUrl(record.conversation_url.clone()))
}

fn session_record_path(state_root: &Path, session_id: &str) -> Result<PathBuf, SessionRecordError> {
    validate_session_id(session_id)
        .map_err(|_| SessionRecordError::Invalid("session id".to_string()))?;
    Ok(state_root
        .join("sessions")
        .join(format!("{session_id}.json")))
}

fn temp_path(path: &Path) -> PathBuf {
    let count = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("session"),
        std::process::id(),
        count,
    ))
}

fn ensure_private_directory(state_root: &Path, path: &Path) -> Result<(), SessionRecordError> {
    crate::provider_runner::create_private_directory(state_root, path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(SessionRecordError::Invalid(
            "unsafe sessions directory".to_string(),
        ));
    }
    Ok(())
}

fn validate_session_file_metadata(file: &File) -> Result<(), SessionRecordError> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(SessionRecordError::Invalid(
            "unsafe session file".to_string(),
        ));
    }
    Ok(())
}

fn verify_session_file(path: &Path, expected: &[u8]) -> Result<(), SessionRecordError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    validate_session_file_metadata(&file)?;
    let mut reopened = Vec::new();
    file.read_to_end(&mut reopened)?;
    if reopened != expected {
        return Err(SessionRecordError::Invalid(
            "session reopen mismatch".to_string(),
        ));
    }
    Ok(())
}

fn now_ms() -> Result<u64, SessionRecordError> {
    Ok(crate::config::now_ms())
}
