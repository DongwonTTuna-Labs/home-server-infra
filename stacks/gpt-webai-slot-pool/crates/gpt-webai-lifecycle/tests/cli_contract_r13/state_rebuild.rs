use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::contracts::cli::LifecycleEnvelope;
use gpt_webai_lifecycle::journal::canonical::canonical_bytes;
use gpt_webai_lifecycle::journal::head::HEAD_SCHEMA;
use gpt_webai_lifecycle::journal::projection::{empty_files, empty_state};
use gpt_webai_lifecycle::journal::{Head, HeadStore, ProjectionStore, ReducedProjection};

#[test]
fn state_rebuild_match_is_read_only_and_emits_closed_envelope() {
    let root = initialized_root("match");
    let before = tree_bytes(&root);
    let output = run(&root, &["--json", "--check-only"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let envelope: LifecycleEnvelope = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(envelope.command, "state-rebuild");
    assert_eq!(envelope.result_kind, "state_rebuild.match");
    assert_eq!(envelope.status, "state_rebuild.match");
    assert!(envelope.ok);
    assert!(envelope.terminal);
    assert!(envelope.reason.is_none());
    assert_eq!(tree_bytes(&root), before);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn state_rebuild_q8_prefers_head_stale_over_ignored_snapshot() {
    let root = initialized_root("head-priority");
    rewrite_head(&root, |head| {
        head.projection_digest = format!("sha256:{}", "1".repeat(64));
        head.snapshot_event_id = Some(format!("evt_{}", "2".repeat(64)));
        head.snapshot_sha256 = Some(format!("sha256:{}", "3".repeat(64)));
    });
    let before = tree_bytes(&root);
    let output = run(&root, &["--json", "--check-only"]);
    assert_eq!(output.status.code(), Some(0));
    let envelope: LifecycleEnvelope = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(envelope.result_kind, "state_rebuild.head_stale");
    assert!(envelope.message.contains("snapshot ignored"));
    assert_eq!(tree_bytes(&root), before);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn state_rebuild_reports_snapshot_ignored_when_head_and_projections_match() {
    let root = initialized_root("snapshot-ignored");
    rewrite_head(&root, |head| {
        head.snapshot_event_id = Some(format!("evt_{}", "4".repeat(64)));
        head.snapshot_sha256 = Some(format!("sha256:{}", "5".repeat(64)));
    });
    let before = tree_bytes(&root);
    let output = run(&root, &["--json", "--check-only"]);
    assert_eq!(output.status.code(), Some(0));
    let envelope: LifecycleEnvelope = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(envelope.result_kind, "state_rebuild.snapshot_ignored");
    assert!(envelope.reason.is_none());
    assert_eq!(tree_bytes(&root), before);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn state_rebuild_failures_and_usage_do_not_mutate_state() {
    let root = initialized_root("digest-mismatch");
    fs::remove_file(root.join("journal/projections/slots.json")).expect("remove projection");
    let before = tree_bytes(&root);
    let output = run(&root, &["--json", "--check-only"]);
    assert_eq!(output.status.code(), Some(70));
    let envelope: LifecycleEnvelope = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(envelope.result_kind, "state_rebuild.digest_mismatch");
    assert_eq!(
        envelope.reason.as_deref(),
        Some("state_rebuild.digest_mismatch")
    );
    assert_eq!(tree_bytes(&root), before);

    let usage = run(&root, &["--check-only", "--json"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert_eq!(tree_bytes(&root), before);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn state_rebuild_reports_lock_contention_without_replay_or_publish() {
    let root = initialized_root("lock-contended");
    let lock = root.join("journal/locks/mutation.lock");
    fs::create_dir(&lock).expect("contended lock");
    fs::set_permissions(lock, fs::Permissions::from_mode(0o700)).expect("private lock mode");
    let before = tree_bytes(&root);

    let output = run(&root, &["--json", "--check-only"]);

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stderr.is_empty());
    let envelope: LifecycleEnvelope = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(envelope.result_kind, "state_rebuild.lock_contended");
    assert_eq!(envelope.reason.as_deref(), Some("lock.contended"));
    assert_eq!(tree_bytes(&root), before);
    fs::remove_dir_all(root).expect("cleanup");
}

fn initialized_root(name: &str) -> PathBuf {
    let root = temp_root(name);
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let head_store = HeadStore::new(&root);
    let guard = head_store.acquire_mutation().expect("guard");
    let files = empty_files();
    let state = empty_state().expect("empty projection");
    let projection = ReducedProjection { state, files };
    let head = Head {
        head_generation: 1,
        last_event_id: None,
        projection_digest: projection.state.projection_digest.clone(),
        schema_version: HEAD_SCHEMA.to_string(),
        snapshot_event_id: None,
        snapshot_sha256: None,
        updated_at_ms: 1,
    };
    head_store.publish(&guard, 0, &head).expect("publish HEAD");
    ProjectionStore::new(&root)
        .publish(&guard, "fixture-init", &projection)
        .expect("publish projections");
    drop(guard);
    root
}

fn rewrite_head(root: &Path, change: impl FnOnce(&mut Head)) {
    let path = root.join("journal/HEAD.json");
    let mut head = HeadStore::new(root)
        .read()
        .expect("read HEAD")
        .expect("HEAD");
    change(&mut head);
    head.validate().expect("valid head shape");
    fs::write(path, canonical_bytes(&head).expect("canonical HEAD")).expect("rewrite HEAD");
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle")))
        .env("GPT_WEBAI_STATE_ROOT", root)
        .arg("state-rebuild")
        .args(args)
        .output()
        .expect("state-rebuild")
}

fn tree_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        let mut entries = fs::read_dir(current)
            .expect("read tree")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let relative = path
                .strip_prefix(root)
                .expect("relative")
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                output.push((format!("d:{relative}"), Vec::new()));
                walk(root, &path, output);
            } else {
                output.push((format!("f:{relative}"), fs::read(&path).expect("read file")));
            }
        }
    }
    let mut output = Vec::new();
    walk(root, root, &mut output);
    output
}

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "gpt-webai-state-rebuild-{name}-{}-{nonce}",
        std::process::id()
    ))
}
