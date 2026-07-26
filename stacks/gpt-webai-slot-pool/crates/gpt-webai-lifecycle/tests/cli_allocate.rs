use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use gpt_webai_lifecycle::records::{holder_count, lock_count};

#[path = "cli_allocate/fixtures.rs"]
mod fixtures;
use fixtures::{binary, stdout_json, Fixture};

#[test]
fn cli_allocate_without_dry_run_is_still_a_read_only_preview() {
    let fixture = Fixture::new("allocate-read-only");
    fixture.write_slot_state("slot-01", "standby");
    let docker = fixture.write_fake_docker("running", 9);

    let output = allocate(&fixture, &docker, false);

    assert!(output.status.success());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["ok"], true);
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["status"], "allocate.dry_run_candidate");
    assert_eq!(value["resultKind"], "allocate.dry_run_candidate");
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(value["requestId"], "request-allocate");
    assert_eq!(value["runId"], "run-allocate");
    assert!(value.get("lockAcquired").is_none());
    assert!(value.get("runtimeStarted").is_none());
    assert_no_mutation(&fixture);
}

#[test]
fn cli_allocate_dry_run_uses_the_same_read_only_path() {
    let fixture = Fixture::new("allocate-dry-run");
    fixture.write_slot_state("slot-01", "standby");
    let docker = fixture.write_fake_docker("running", 9);

    let output = allocate(&fixture, &docker, true);

    assert!(output.status.success());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["ok"], true);
    assert_eq!(value["status"], "allocate.dry_run_candidate");
    assert_no_mutation(&fixture);
}

#[test]
fn cli_allocate_rejects_runtime_start_timeout_before_docker_or_state_mutation() {
    let fixture = Fixture::new("allocate-reject-runtime-timeout");
    fixture.write_slot_state("slot-01", "standby");
    let docker = fixture.write_fake_docker("running", 0);

    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .args([
            "allocate",
            "--json",
            "--request-id",
            "request-allocate",
            "--run-id",
            "run-allocate",
            "--fencing-token",
            "token-allocate",
            "--docker-bin",
        ])
        .arg(&docker)
        .args(["--runtime-start-timeout-ms", "30000"])
        .output()
        .expect("allocate");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("read-only R13"));
    assert!(!fixture.docker_log.exists());
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
    assert_eq!(fixture.slot_state("slot-01"), "status=standby\n");
}

#[test]
fn cli_allocate_requires_identifiers_even_for_dry_run() {
    let fixture = Fixture::new("allocate-requires-identifiers");
    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .args(["allocate", "--json", "--dry-run"])
        .output()
        .expect("allocate");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--request-id"));
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
}

#[test]
fn cli_allocate_reports_lock_contention_after_read_only_probe() {
    let fixture = Fixture::new("allocate-lock-contended");
    fixture.write_slot_state("slot-01", "standby");
    for relative in ["journal", "journal/locks", "journal/locks/mutation.lock"] {
        let path = fixture.root.join(relative);
        fs::create_dir(&path).expect("private contended lock component");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("private contended lock mode");
    }
    let docker = fixture.write_fake_docker("running", 9);

    let output = allocate(&fixture, &docker, true);

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stderr.is_empty());
    let value = stdout_json(&output.stdout);
    assert_eq!(value["resultKind"], "allocate.lock_contended");
    assert_eq!(value["reason"], "lock.contended");
    assert_eq!(fixture.slot_state("slot-01"), "status=standby\n");
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
}

fn allocate(fixture: &Fixture, docker: &std::path::Path, dry_run: bool) -> std::process::Output {
    let mut command = Command::new(binary());
    command
        .env("GPT_WEBAI_STATE_ROOT", &fixture.root)
        .env("GPT_WEBAI_SLOT_COUNT", "1")
        .env("GPT_WEBAI_SLOT_MODE", "docker")
        .env("GPT_WEBAI_RUST_STATUS_PROVIDER_CHECK", "0")
        .args([
            "allocate",
            "--json",
            "--request-id",
            "request-allocate",
            "--run-id",
            "run-allocate",
            "--fencing-token",
            "token-allocate",
            "--docker-bin",
        ])
        .arg(docker);
    if dry_run {
        command.arg("--dry-run");
    }
    command.output().expect("allocate")
}

fn assert_no_mutation(fixture: &Fixture) {
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
    assert_eq!(fixture.slot_state("slot-01"), "status=standby\n");
    let log = fs::read_to_string(&fixture.docker_log).unwrap_or_default();
    assert!(log.contains("inspect -f {{.State.Status}} gpt-webai-slot-01"));
    assert!(!log.contains("start gpt-webai-slot-01"));
    assert!(!log.contains("stop gpt-webai-slot-01"));
}
