use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::slots::{next_preferred_group, AccountGroupId};

const GROUP_CURSOR_SCHEMA: &str = "gpt-webai.group-cursor.v2";
const SLOT_ROTATION_CURSOR_SCHEMA: &str = "gpt-webai.slot-rotation-cursor.v1";

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupCursorRecord {
    pub schema: String,
    pub last_preferred_group: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotRotationCursorRecord {
    pub schema: String,
    pub account_group: String,
    pub last_allocated_slot: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationCursorUpdate<'a> {
    pub preferred_group: &'a AccountGroupId,
    pub allocated_group: &'a AccountGroupId,
    pub slot_id: &'a str,
}

pub fn read_key_value_file(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(values)
}

pub fn read_group_cursor(state_root: &Path) -> std::io::Result<Option<GroupCursorRecord>> {
    let path = group_cursor_path(state_root);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let record = serde_json::from_str::<GroupCursorRecord>(&text).map_err(std::io::Error::other)?;
    if record.schema != GROUP_CURSOR_SCHEMA {
        return Ok(None);
    }
    Ok(Some(record))
}

pub fn next_preferred_group_from_cursor(state_root: &Path) -> std::io::Result<AccountGroupId> {
    let cursor = read_group_cursor(state_root)?;
    Ok(next_preferred_group(
        cursor
            .as_ref()
            .map(|record| record.last_preferred_group.as_str()),
    ))
}

pub fn write_group_cursor(
    state_root: &Path,
    preferred_group: &AccountGroupId,
) -> std::io::Result<GroupCursorRecord> {
    let record = GroupCursorRecord {
        schema: GROUP_CURSOR_SCHEMA.to_string(),
        last_preferred_group: preferred_group.0.clone(),
    };
    let path = group_cursor_path(state_root);
    let parent = path.parent().expect("group cursor path has parent");
    crate::provider_runner::create_private_directory(state_root, parent)?;
    let tmp = atomic_temp_path(&path, "group-cursor.json");
    fs::write(
        &tmp,
        format!("{}\n", serde_json::to_string_pretty(&record)?),
    )?;
    fs::rename(&tmp, &path)?;
    Ok(record)
}

pub fn read_slot_rotation_cursors(
    state_root: &Path,
    groups: impl IntoIterator<Item = String>,
) -> std::io::Result<BTreeMap<String, String>> {
    let mut cursors = BTreeMap::new();
    for group in groups {
        if cursors.contains_key(&group) {
            continue;
        }
        let Some(record) = read_slot_rotation_cursor(state_root, &group)? else {
            continue;
        };
        cursors.insert(record.account_group, record.last_allocated_slot);
    }
    Ok(cursors)
}

pub fn read_slot_rotation_cursor(
    state_root: &Path,
    account_group: &str,
) -> std::io::Result<Option<SlotRotationCursorRecord>> {
    let path = slot_rotation_cursor_path(state_root, account_group);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let record =
        serde_json::from_str::<SlotRotationCursorRecord>(&text).map_err(std::io::Error::other)?;
    if record.schema != SLOT_ROTATION_CURSOR_SCHEMA || record.account_group != account_group {
        return Ok(None);
    }
    Ok(Some(record))
}

pub fn write_slot_rotation_cursor(
    state_root: &Path,
    account_group: &AccountGroupId,
    slot_id: &str,
) -> std::io::Result<SlotRotationCursorRecord> {
    let record = SlotRotationCursorRecord {
        schema: SLOT_ROTATION_CURSOR_SCHEMA.to_string(),
        account_group: account_group.0.clone(),
        last_allocated_slot: slot_id.to_string(),
    };
    let path = slot_rotation_cursor_path(state_root, &account_group.0);
    let parent = path.parent().expect("slot rotation cursor path has parent");
    crate::provider_runner::create_private_directory(state_root, parent)?;
    let tmp = atomic_temp_path(&path, "slot-rotation-cursor.json");
    fs::write(
        &tmp,
        format!("{}\n", serde_json::to_string_pretty(&record)?),
    )?;
    fs::rename(&tmp, &path)?;
    Ok(record)
}

pub fn write_allocation_cursors(
    state_root: &Path,
    update: AllocationCursorUpdate<'_>,
) -> std::io::Result<()> {
    write_group_cursor(state_root, update.preferred_group).and_then(|_| {
        write_slot_rotation_cursor(state_root, update.allocated_group, update.slot_id).map(|_| ())
    })
}

pub fn advance_group_cursor(state_root: &Path) -> std::io::Result<GroupCursorRecord> {
    let preferred_group = next_preferred_group_from_cursor(state_root)?;
    write_group_cursor(state_root, &preferred_group)
}

pub fn holder_count(state_root: &Path) -> usize {
    count_entries(state_root.join("holders").as_path())
}

pub fn lock_count(state_root: &Path) -> usize {
    let slot_locks = state_root.join("locks").join("slots");
    fs::read_dir(slot_locks)
        .map(|iter| {
            iter.filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| name.ends_with(".lock"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn count_entries(path: &Path) -> usize {
    fs::read_dir(path).map(|iter| iter.count()).unwrap_or(0)
}

fn atomic_temp_path(path: &Path, fallback_name: &str) -> PathBuf {
    let parent = path.parent().expect("atomic path has parent");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback_name);
    let counter = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.tmp.{}-{}-{counter}",
        std::process::id(),
        now_nanos()
    ))
}

fn now_nanos() -> u128 {
    u128::from(crate::config::now_ms()) * 1_000_000
}

fn group_cursor_path(state_root: &Path) -> std::path::PathBuf {
    state_root.join("slots").join("group-cursor.json")
}

fn slot_rotation_cursor_path(state_root: &Path, account_group: &str) -> std::path::PathBuf {
    state_root
        .join("slots")
        .join(format!("{account_group}-slot-cursor.json"))
}
