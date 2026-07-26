use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::records::{
    advance_group_cursor, next_preferred_group_from_cursor, read_group_cursor,
    read_slot_rotation_cursor, read_slot_rotation_cursors, write_group_cursor,
    write_slot_rotation_cursor,
};
use gpt_webai_lifecycle::slots::AccountGroupId;

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

#[test]
fn missing_group_cursor_starts_with_group_01() {
    let root = temp_state_root("missing-cursor");
    let preferred = next_preferred_group_from_cursor(&root).expect("cursor read");
    assert_eq!(preferred.0, "group-01");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn group_cursor_persists_last_preferred_group_atomically() {
    let root = temp_state_root("write-cursor");
    let written =
        write_group_cursor(&root, &AccountGroupId("group-02".to_string())).expect("write cursor");
    let read = read_group_cursor(&root)
        .expect("read cursor")
        .expect("cursor exists");

    assert_eq!(written, read);
    assert_eq!(read.schema, "gpt-webai.group-cursor.v2");
    assert_eq!(read.last_preferred_group, "group-02");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn advancing_group_cursor_alternates_persisted_preference() {
    let root = temp_state_root("advance-cursor");

    let first = advance_group_cursor(&root).expect("first advance");
    let second = advance_group_cursor(&root).expect("second advance");
    let third = advance_group_cursor(&root).expect("third advance");

    assert_eq!(first.last_preferred_group, "group-01");
    assert_eq!(second.last_preferred_group, "group-02");
    assert_eq!(third.last_preferred_group, "group-01");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn slot_rotation_cursor_persists_last_allocated_slot_per_group() {
    let root = temp_state_root("slot-rotation-cursor");
    let group = AccountGroupId("group-02".to_string());

    let written = write_slot_rotation_cursor(&root, &group, "slot-08").expect("write slot cursor");
    let read = read_slot_rotation_cursor(&root, "group-02")
        .expect("read slot cursor")
        .expect("slot cursor exists");

    assert_eq!(written, read);
    assert_eq!(read.schema, "gpt-webai.slot-rotation-cursor.v1");
    assert_eq!(read.account_group, "group-02");
    assert_eq!(read.last_allocated_slot, "slot-08");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn slot_rotation_cursor_map_ignores_missing_groups() {
    let root = temp_state_root("slot-rotation-map");
    write_slot_rotation_cursor(&root, &AccountGroupId("group-01".to_string()), "slot-03")
        .expect("write slot cursor");

    let cursors =
        read_slot_rotation_cursors(&root, ["group-01".to_string(), "group-02".to_string()])
            .expect("read cursor map");

    assert_eq!(cursors.get("group-01").map(String::as_str), Some("slot-03"));
    assert!(!cursors.contains_key("group-02"));
    let _ = fs::remove_dir_all(root);
}
