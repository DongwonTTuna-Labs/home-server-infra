use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::locks::{
    acquire_slot_lease, read_slot_lease, release_slot_lease, sha256_hex, LockError,
};
use gpt_webai_lifecycle::records::{holder_count, lock_count};

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
fn acquire_slot_lease_creates_fenced_lock_and_holder_without_raw_token() {
    let root = temp_state_root("lease-acquire");
    let record = acquire_slot_lease(
        &root,
        "slot-01",
        "request-a",
        "run-a",
        "secret-fencing-token",
        30_000,
    )
    .expect("acquire lease");

    assert_eq!(record.slot_id, "slot-01");
    assert_eq!(record.request_id, "request-a");
    assert_eq!(
        record.fencing_token_sha256,
        sha256_hex("secret-fencing-token")
    );
    let stored =
        fs::read_to_string(root.join("locks/slots/slot-01.lock/lease.json")).expect("stored lease");
    assert!(!stored.contains("secret-fencing-token"));
    assert!(root.join("holders/slot-01.holder.json").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn second_acquire_of_same_slot_is_busy_until_fenced_release() {
    let root = temp_state_root("lease-busy");
    acquire_slot_lease(&root, "slot-02", "request-a", "run-a", "token-a", 30_000)
        .expect("first lease");

    let busy = acquire_slot_lease(&root, "slot-02", "request-b", "run-b", "token-b", 30_000)
        .expect_err("second lease is busy");
    assert!(matches!(busy, LockError::Busy(slot) if slot == "slot-02"));

    release_slot_lease(&root, "slot-02", "token-a").expect("release first lease");
    acquire_slot_lease(&root, "slot-02", "request-b", "run-b", "token-b", 30_000)
        .expect("acquire after release");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn wrong_fencing_token_cannot_release_slot_lease() {
    let root = temp_state_root("lease-fencing");
    acquire_slot_lease(&root, "slot-03", "request-a", "run-a", "token-a", 30_000)
        .expect("first lease");

    let error = release_slot_lease(&root, "slot-03", "token-b").expect_err("wrong token");
    assert!(matches!(error, LockError::FencingMismatch(slot) if slot == "slot-03"));
    assert_eq!(
        read_slot_lease(&root, "slot-03")
            .expect("lease remains")
            .request_id,
        "request-a"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lock_and_holder_counts_ignore_empty_parent_directories_after_release() {
    let root = temp_state_root("lease-counts");
    acquire_slot_lease(&root, "slot-04", "request-a", "run-a", "token-a", 30_000).expect("lease");
    assert_eq!(holder_count(&root), 1);
    assert_eq!(lock_count(&root), 1);

    release_slot_lease(&root, "slot-04", "token-a").expect("release");
    assert_eq!(holder_count(&root), 0);
    assert_eq!(lock_count(&root), 0);
    assert!(root.join("locks/slots").exists());
    let _ = fs::remove_dir_all(root);
}
