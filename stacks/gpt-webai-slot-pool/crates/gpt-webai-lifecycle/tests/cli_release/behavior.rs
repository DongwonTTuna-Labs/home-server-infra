use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use gpt_webai_lifecycle::contracts::events::EventType;
use gpt_webai_lifecycle::journal::EventStore;

use super::fixtures::{binary, stdout_json, Fixture};

#[test]
fn fenced_release_stops_owned_runtime_and_releases_every_resource() {
    let fixture = Fixture::new("fenced-release");
    fixture.seed_active_session("sid-release", "token-release");
    let docker_log = fixture.root.join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);

    let output = release(
        &fixture,
        "sid-release",
        Some("token-release"),
        Some(&docker_bin),
        true,
    );

    assert_eq!(output.status.code(), Some(0));
    let value = stdout_json(&output.stdout);
    assert_r13(&value, "release.allocatable", true);
    assert_eq!(value["sessionId"], "sid-release");
    assert_eq!(value["slotId"], "slot-01");
    assert_eq!(
        event_types_for_output(&fixture, &value),
        vec![
            EventType::ReleaseStarted,
            EventType::ReleaseEvidencePreserved,
            EventType::RuntimeStopStarted,
            EventType::RuntimeStopped,
            EventType::ReleaseCleanupStarted,
            EventType::SessionOperationClaimReleased,
            EventType::SlotLeaseReleased,
            EventType::RuntimeOwnershipReleased,
            EventType::ReleaseCleanupCommitted,
            EventType::SlotStandbyWritten,
            EventType::ReleaseFinalized,
        ]
    );
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker log"),
        concat!(
            "inspect gpt-webai-slot-01\n",
            "compose -p gpt-webai-slot-pool stop gpt-webai-slot-01\n",
            "inspect gpt-webai-slot-01\n"
        )
    );
}

#[test]
fn wrong_fence_fails_before_release_started_or_docker() {
    let fixture = Fixture::new("wrong-fence");
    fixture.seed_active_session("sid-wrong-fence", "token-release");
    let docker_log = fixture.root.join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);
    let before = EventStore::new(&fixture.root)
        .load_all()
        .expect("events before")
        .len();

    let output = release(
        &fixture,
        "sid-wrong-fence",
        Some("wrong-token"),
        Some(&docker_bin),
        false,
    );

    assert_eq!(output.status.code(), Some(70));
    let value = stdout_json(&output.stdout);
    assert_r13(&value, "release.fencing_mismatch", false);
    assert!(value["eventIds"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        EventStore::new(&fixture.root)
            .load_all()
            .expect("events after")
            .len(),
        before
    );
    assert!(!docker_log.exists());
}

#[test]
fn tokenless_live_owner_is_preserved_while_local_claim_and_lease_are_released() {
    let fixture = Fixture::new("owner-alive");
    fixture.seed_active_session("sid-owner-alive", "token-release");
    let docker_log = fixture.root.join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);

    let output = release(&fixture, "sid-owner-alive", None, Some(&docker_bin), false);

    assert_eq!(output.status.code(), Some(0));
    let value = stdout_json(&output.stdout);
    assert_r13(&value, "release.stop_skipped_owner_alive", true);
    let types = event_types_for_output(&fixture, &value);
    assert!(types.contains(&EventType::RuntimeStopSkipped));
    assert!(types.contains(&EventType::SessionOperationClaimReleased));
    assert!(types.contains(&EventType::SlotLeaseReleased));
    assert!(!types.contains(&EventType::RuntimeOwnershipReleased));
    assert!(types.contains(&EventType::ReleaseCleanupCommitted));
    assert!(types.contains(&EventType::ReleaseFinalized));
    assert!(!docker_log.exists());
}

#[test]
fn stop_failure_still_cleans_local_resources_and_finalizes_nonallocatable() {
    let fixture = Fixture::new("stop-failure");
    fixture.seed_active_session("sid-stop-failure", "token-release");
    let docker_log = fixture.root.join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 9);

    let output = release(
        &fixture,
        "sid-stop-failure",
        Some("token-release"),
        Some(&docker_bin),
        false,
    );

    assert_eq!(output.status.code(), Some(70));
    let value = stdout_json(&output.stdout);
    assert_r13(&value, "release.stop_failed", false);
    let types = event_types_for_output(&fixture, &value);
    assert!(types.contains(&EventType::RuntimeStopFailed));
    assert!(types.contains(&EventType::SessionOperationClaimReleased));
    assert!(types.contains(&EventType::SlotLeaseReleased));
    assert!(types.contains(&EventType::RuntimeOwnershipReleased));
    assert!(types.contains(&EventType::ReleaseCleanupCommitted));
    assert!(types.contains(&EventType::ReleaseFinalized));
    assert_eq!(
        std::fs::read_to_string(docker_log).expect("docker log"),
        concat!(
            "inspect gpt-webai-slot-01\n",
            "compose -p gpt-webai-slot-pool stop gpt-webai-slot-01\n"
        )
    );

    let request_root = fixture
        .root
        .join("evidence/requests/r-request-sid-stop-failure");
    let operations_root = request_root.join("operations");
    let release_roots = std::fs::read_dir(&operations_root)
        .expect("release evidence operations")
        .map(|entry| entry.expect("release evidence entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "release")
        })
        .collect::<Vec<_>>();
    assert_eq!(release_roots.len(), 1, "one release evidence root");
    for path in [
        fixture.root.clone(),
        fixture.root.join("evidence"),
        fixture.root.join("evidence/requests"),
        request_root,
        operations_root,
        release_roots[0].clone(),
    ] {
        let mode = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("metadata for {}: {error}", path.display()))
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "private directory {}", path.display());
    }
}

#[test]
fn unknown_session_is_a_closed_failure_without_mutation() {
    let fixture = Fixture::new("unknown-session");
    let before = EventStore::new(&fixture.root)
        .load_all()
        .expect("events before")
        .len();

    let output = release(&fixture, "missing", Some("token"), None, false);

    assert_eq!(output.status.code(), Some(70));
    let value = stdout_json(&output.stdout);
    assert_r13(&value, "release.target_unknown", false);
    assert_eq!(value["sessionId"], "missing");
    assert_eq!(
        EventStore::new(&fixture.root)
            .load_all()
            .expect("events after")
            .len(),
        before
    );
}

#[test]
fn a_second_release_returns_already_released_without_duplicate_events() {
    let fixture = Fixture::new("already-released");
    fixture.seed_active_session("sid-already", "token-release");
    let docker_log = fixture.root.join("docker-args.txt");
    let docker_bin = fixture.write_fake_docker(&docker_log, 0);
    let first = release(
        &fixture,
        "sid-already",
        Some("token-release"),
        Some(&docker_bin),
        false,
    );
    assert_eq!(first.status.code(), Some(0));
    let before = EventStore::new(&fixture.root)
        .load_all()
        .expect("events before second")
        .len();

    let second = release(
        &fixture,
        "sid-already",
        Some("token-release"),
        Some(&docker_bin),
        false,
    );

    assert_eq!(second.status.code(), Some(0));
    let value = stdout_json(&second.stdout);
    assert_r13(&value, "release.already_released", true);
    assert!(value["eventIds"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        EventStore::new(&fixture.root)
            .load_all()
            .expect("events after second")
            .len(),
        before
    );
}

fn release(
    fixture: &Fixture,
    session_id: &str,
    token: Option<&str>,
    docker_bin: Option<&Path>,
    stop_runtime_flag: bool,
) -> Output {
    let mut command = Command::new(binary());
    command.env("GPT_WEBAI_STATE_ROOT", &fixture.root).args([
        "release",
        "--json",
        "--session",
        session_id,
    ]);
    if let Some(token) = token {
        command.args(["--fencing-token", token]);
    }
    if stop_runtime_flag {
        command.arg("--stop-runtime");
    }
    if let Some(docker_bin) = docker_bin {
        command.arg("--docker-bin").arg(docker_bin);
    }
    command.output().expect("run release")
}

fn assert_r13(value: &serde_json::Value, result_kind: &str, ok: bool) {
    assert_eq!(value["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(value["command"], "release");
    assert_eq!(value["resultKind"], result_kind);
    assert_eq!(value["status"], result_kind);
    assert_eq!(value["ok"], ok);
}

fn event_types_for_output(fixture: &Fixture, value: &serde_json::Value) -> Vec<EventType> {
    let events = EventStore::new(&fixture.root)
        .load_all()
        .expect("load journal");
    value["eventIds"]
        .as_array()
        .expect("event ids")
        .iter()
        .map(|id| {
            let id = id.as_str().expect("event id string");
            events
                .iter()
                .find(|event| event.event_id == id)
                .unwrap_or_else(|| panic!("missing event {id}"))
                .event_type
        })
        .collect()
}
