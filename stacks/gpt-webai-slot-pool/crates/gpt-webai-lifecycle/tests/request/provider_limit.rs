use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::{ready_runtime, standby_exited_runtime, FakeRun, InputSpec};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::request::provider_limit::default_provider_limit_retry_delays;
use gpt_webai_lifecycle::request::run::run_provider_round_trip;
use gpt_webai_lifecycle::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use gpt_webai_lifecycle::sessions::read_session_record;

#[test]
fn default_provider_limit_retry_delays_are_two_fifteen_minute_cooldowns() {
    let seconds = default_provider_limit_retry_delays()
        .iter()
        .map(Duration::as_secs)
        .collect::<Vec<_>>();

    assert_eq!(seconds, vec![900, 900]);
}

#[test]
fn provider_limit_after_session_creation_stays_pinned_without_resend() {
    let mut fixture = FakeRun::new("poll-provider-limit-other-group");
    fixture.provider = write_provider_limit_sequence(fixture.path(), &fixture.args_log, 1);
    let input = input(&fixture);

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok, "{output:?}");
    assert_eq!(output.reason.as_deref(), Some("provider.limit"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(output.session_id.as_deref(), Some("sid-1"));
    assert_eq!(output.send_attempts, 1);
    assert!(output.provider_limit_retry_delays_ms.is_empty());
    assert_clean(&fixture);
    let slot_01_state = read_state(&fixture, "slot-01").expect("slot-01 state");
    assert!(slot_01_state.contains("status=provider.limit\n"));
    assert!(slot_01_state.contains("provider_limit_next_retry_at_ms="));
    let pinned = read_session_record(fixture.path(), "sid-1").expect("pinned session");
    assert_eq!(pinned.session_id, "sid-1");
    assert_eq!(pinned.slot_id, "slot-01");
    assert!(pinned.updated_at_ms >= pinned.created_at_ms);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 1);
}

#[test]
fn provider_limit_after_session_creation_does_not_start_a_cooldown_round() {
    let mut fixture = FakeRun::new("poll-provider-limit-cooldown-round");
    fixture.provider = write_provider_limit_sequence(fixture.path(), &fixture.args_log, 2);
    let mut input = input(&fixture);
    input.provider_limit_retry_delays = vec![Duration::ZERO];

    let output = run_provider_round_trip(input, &ready_runtime());

    assert!(!output.ok, "{output:?}");
    assert_eq!(output.reason.as_deref(), Some("provider.limit"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(output.session_id.as_deref(), Some("sid-1"));
    assert_eq!(output.send_attempts, 1);
    assert!(output.provider_limit_retry_delays_ms.is_empty());
    assert_clean(&fixture);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 1);
}

#[test]
fn provider_limit_after_session_creation_releases_runtime_without_reopening_slots() {
    let mut fixture = FakeRun::new("poll-provider-limit-persisted-cooldown");
    fixture.provider = write_provider_limit_sequence(fixture.path(), &fixture.args_log, 4);
    write_state(&fixture, "slot-01", "standby");
    write_state(&fixture, "slot-02", "standby");
    let docker_log = fixture.path().join("docker.log");
    let docker = fixture.write_fake_docker(&docker_log, 0);
    let mut input = input(&fixture);
    input.runtime_start_mode = RuntimeStartMode::docker(docker.clone(), Duration::from_secs(1));
    input.runtime_release_mode = RuntimeReleaseMode::docker(docker, Duration::from_secs(1));
    input.provider_limit_retry_delays = vec![Duration::ZERO, Duration::ZERO];

    let output = run_provider_round_trip(input, &standby_exited_runtime());

    assert!(!output.ok, "{output:?}");
    assert_eq!(output.reason.as_deref(), Some("provider.limit"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-01"));
    assert_eq!(output.session_id.as_deref(), Some("sid-1"));
    assert_eq!(output.send_attempts, 1);
    assert!(output.provider_limit_retry_delays_ms.is_empty());
    assert!(output.runtime_stopped);
    assert_clean(&fixture);
    let args = fs::read_to_string(&fixture.args_log).expect("args log");
    assert_eq!(command_count(&args, "send "), 1);
    assert_eq!(command_count(&args, "poll "), 1);
}

fn input(fixture: &FakeRun) -> gpt_webai_lifecycle::request::run::RequestRunInput {
    let unused_send = fixture.write_file("unused-send.json", "{}");
    let unused_poll = fixture.write_file("unused-poll.json", "{}");
    fixture.input(InputSpec {
        send_json: unused_send,
        poll_json: unused_poll,
        download_json: None,
        files: Vec::new(),
    })
}

fn write_provider_limit_sequence(root: &Path, args_log: &Path, limit_polls: u8) -> PathBuf {
    let path = root.join("provider-limit-sequence.sh");
    let send_count = root.join("send.count");
    let poll_count = root.join("poll.count");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\n\
             printf '%s ' \"$@\" >> '{}'\n\
             printf '\\n' >> '{}'\n\
             case \"$1\" in\n\
             send)\n\
               count=0\n\
               [ -f '{}' ] && count=$(cat '{}')\n\
               count=$((count + 1))\n\
               printf '%s\\n' \"$count\" > '{}'\n\
               sid=\"sid-$count\"\n\
               printf '{{\"schema\":\"{}\",\"ok\":true,\"vendor\":\"chatgpt\",\"status\":\"sent\",\"sessionId\":\"%s\",\"targetId\":\"target-run\",\"conversationUrl\":\"https://chatgpt.com/c/%s\",\"turnEvidence\":{{\"activeTurn\":true,\"userTurnId\":\"turn_{}\",\"assistantTurnId\":\"turn_{}\"}}}}\\n' \"$sid\" \"$sid\"\n\
               ;;\n\
             poll)\n\
               count=0\n\
               [ -f '{}' ] && count=$(cat '{}')\n\
               count=$((count + 1))\n\
               printf '%s\\n' \"$count\" > '{}'\n\
               sid=\"sid-$count\"\n\
               if [ \"$count\" -le '{}' ]; then\n\
                 printf '{{\"schema\":\"{}\",\"ok\":true,\"vendor\":\"chatgpt\",\"status\":\"provider_limit\",\"reason\":\"provider.limit\",\"sessionId\":\"%s\",\"targetId\":\"target-run\",\"conversationUrl\":\"https://chatgpt.com/c/%s\",\"answerText\":\"limited answer evidence\",\"assistantTurn\":{{\"textSha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}}}\\n' \"$sid\" \"$sid\"\n\
               else\n\
                 printf '{{\"schema\":\"{}\",\"ok\":true,\"vendor\":\"chatgpt\",\"status\":\"done\",\"sessionId\":\"%s\",\"targetId\":\"target-run\",\"conversationUrl\":\"https://chatgpt.com/c/%s\",\"answerText\":\"final answer\",\"assistantTurn\":{{\"textSha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}}}\\n' \"$sid\" \"$sid\"\n\
               fi\n\
               ;;\n\
             *) printf '{{\"schema\":\"{}\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}}\\n'; exit 2 ;;\n\
             esac\n",
            args_log.display(),
            args_log.display(),
            send_count.display(),
            send_count.display(),
            send_count.display(),
            PROVIDER_SCHEMA,
            "1".repeat(64),
            "2".repeat(64),
            poll_count.display(),
            poll_count.display(),
            poll_count.display(),
            limit_polls,
            PROVIDER_SCHEMA,
            PROVIDER_SCHEMA,
            PROVIDER_SCHEMA
        ),
    )
    .expect("write provider limit sequence");
    set_executable(&path);
    path
}

fn assert_clean(fixture: &FakeRun) {
    assert_eq!(holder_count(fixture.path()), 0);
    assert_eq!(lock_count(fixture.path()), 0);
}

fn read_state(fixture: &FakeRun, slot_id: &str) -> Option<String> {
    fs::read_to_string(
        fixture
            .path()
            .join("slots")
            .join(format!("{slot_id}.state")),
    )
    .ok()
}

fn write_state(fixture: &FakeRun, slot_id: &str, status: &str) {
    let path = fixture
        .path()
        .join("slots")
        .join(format!("{slot_id}.state"));
    let parent = path.parent().expect("slot state parent");
    fs::create_dir_all(parent).expect("slot state dir");
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).expect("private slot state dir");
    fs::write(&path, format!("status={status}\n")).expect("slot state");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private slot state");
}

fn command_count(args: &str, prefix: &str) -> usize {
    args.lines().filter(|line| line.starts_with(prefix)).count()
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}
