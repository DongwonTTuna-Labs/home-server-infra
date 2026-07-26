use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::contracts::cli::{result_spec, LifecycleEnvelope, ResultSpec};

#[path = "cli_contract_r13/state_rebuild.rs"]
mod state_rebuild;

#[test]
fn lifecycle_json_stdout_is_sorted_canonical_json_plus_lf() {
    let root = temp_root("canonical-stdout");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .args(["cleanup", "--json", "--dry-run"])
        .output()
        .expect("cleanup");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let expected =
        gpt_webai_lifecycle::journal::canonical::canonical_bytes(&value).expect("canonical bytes");
    assert_eq!(output.stdout, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lexical_contract_rejects_invalid_utf8_assignment_separator_and_double_dash() {
    let invalid_utf8 = run_os([OsString::from("status"), OsString::from_vec(vec![0xff])]);
    assert_usage(invalid_utf8, "valid UTF-8");
    assert_usage(run(["status", "--json=true"]), "separate tokens");
    assert_usage(run(["status", "--"]), "does not accept --");
}

#[test]
fn unknown_failpoint_is_usage_error_before_any_state_work() {
    let root = temp_root("unknown-failpoint");
    assert!(!root.exists());
    let output = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .env("GPT_WEBAI_FAILPOINT", "after-unrecognised-effect")
        .args(["cleanup", "--json", "--dry-run"])
        .output()
        .expect("unknown failpoint");
    assert_usage(output, "unrecognised GPT_WEBAI_FAILPOINT");
    assert!(!root.exists(), "unknown failpoint must not create state");
}

#[test]
fn lexical_contract_rejects_duplicates_missing_values_and_positionals() {
    assert_usage(run(["status", "--json", "--json"]), "duplicate singleton");
    assert_usage(run(["status", "--json", "--legacy-kv"]), "at most one");
    assert_usage(run(["show", "--session"]), "missing value");
    assert_usage(
        run(["show", "--session", "--json"]),
        "missing value for --session",
    );
    assert_usage(run(["constants", "extra"]), "unexpected positional");
    assert_usage(run(["cleanup", "--dry-run"]), "requires --json");
}

#[test]
fn lexical_contract_preserves_single_dash_values_and_caps_repeated_files() {
    let root = temp_root("files");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let prompt = root.join("prompt.txt");
    fs::write(&prompt, "prompt").expect("prompt");
    let mut command = Command::new(binary());
    command.args(["run", "--kind", "pro"]);
    for _ in 0..65 {
        command.arg("--file").arg(&prompt);
    }
    command.args(["--prompt", "-single-dash-value"]);
    assert_usage(command.output().expect("run"), "no more than 64");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn preflight_requires_and_accepts_the_exact_fake_provider_bundle() {
    let root = temp_root("preflight-bundle");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let log = root.join("provider.log");
    let provider = write_preflight_provider(&root, &log);

    let accepted = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .args([
            "preflight",
            "--json",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--run-id", "run-preflight"])
        .output()
        .expect("preflight");
    assert!(
        accepted.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&accepted.stdout).expect("json");
    assert_eq!(value["ok"], true, "stdout={value}");
    assert_eq!(value["runId"], "run-preflight");

    let incomplete = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &root)
        .args(["preflight", "--json", "--fake-provider", "--provider-bin"])
        .arg(&provider)
        .args(["--run-id", "run-incomplete"])
        .output()
        .expect("preflight");
    assert_usage(incomplete, "exact bundle");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn preflight_state_failures_emit_closed_r13_envelopes() {
    let provider = write_preflight_provider(
        std::path::Path::new("."),
        std::path::Path::new("provider.log"),
    );

    let corrupt_root = temp_root("preflight-corrupt-cursor");
    let slots = corrupt_root.join("slots");
    fs::create_dir_all(&slots).expect("slots");
    fs::set_permissions(&corrupt_root, fs::Permissions::from_mode(0o700)).expect("private root");
    fs::set_permissions(&slots, fs::Permissions::from_mode(0o700)).expect("private slots");
    let cursor = slots.join("group-cursor.json");
    fs::write(&cursor, "{\n").expect("corrupt cursor");
    fs::set_permissions(&cursor, fs::Permissions::from_mode(0o600)).expect("private cursor");
    let state_error = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &corrupt_root)
        .args([
            "preflight",
            "--json",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--run-id", "run-preflight-state-error"])
        .output()
        .expect("preflight state error");
    assert_eq!(state_error.status.code(), Some(70));
    assert!(state_error.stderr.is_empty());
    let state_envelope: serde_json::Value =
        serde_json::from_slice(&state_error.stdout).expect("state-error envelope");
    assert_eq!(state_envelope["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(state_envelope["resultKind"], "preflight.state_invalid");
    assert_eq!(state_envelope["status"], "preflight.state_invalid");
    assert_eq!(state_envelope["ok"], false);
    assert_eq!(state_envelope["terminal"], true);
    assert_eq!(state_envelope["reason"], "journal.immutable_collision");
    assert_eq!(state_envelope["runId"], "run-preflight-state-error");
    assert!(state_envelope["slotId"].is_null());
    assert!(state_envelope["cohort"].is_null());

    let guard_root = temp_root("preflight-invalid-root-mode");
    fs::create_dir_all(&guard_root).expect("guard root");
    fs::set_permissions(&guard_root, fs::Permissions::from_mode(0o500)).expect("invalid root mode");
    let guard_error = Command::new(binary())
        .env("GPT_WEBAI_STATE_ROOT", &guard_root)
        .args([
            "preflight",
            "--json",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--slot", "slot-01", "--run-id", "run-preflight-guard-error"])
        .output()
        .expect("preflight guard error");
    assert_eq!(guard_error.status.code(), Some(70));
    assert!(guard_error.stderr.is_empty());
    let guard_envelope: serde_json::Value =
        serde_json::from_slice(&guard_error.stdout).expect("guard-error envelope");
    assert_eq!(guard_envelope["schema"], "gpt-webai.lifecycle.r13.v1");
    assert_eq!(guard_envelope["resultKind"], "preflight.state_invalid");
    assert_eq!(guard_envelope["status"], "preflight.state_invalid");
    assert_eq!(guard_envelope["ok"], false);
    assert_eq!(guard_envelope["terminal"], true);
    assert_eq!(guard_envelope["reason"], "journal.immutable_collision");
    assert_eq!(guard_envelope["runId"], "run-preflight-guard-error");
    assert_eq!(guard_envelope["slotId"], "slot-01");
    assert_eq!(guard_envelope["cohort"], "cohort-a");

    let _ = fs::remove_dir_all(corrupt_root);
    fs::set_permissions(&guard_root, fs::Permissions::from_mode(0o700))
        .expect("restore guard root mode");
    let _ = fs::remove_dir_all(guard_root);
}

#[test]
fn modern_run_rejects_invalid_model_effort_and_fake_live_gates_before_provider() {
    let root = temp_root("model-effort");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let prompt = root.join("prompt.txt");
    fs::write(&prompt, "prompt").expect("prompt");
    let provider = write_executable(&root, "provider.sh", "#!/usr/bin/env bash\nexit 9\n");

    for (model, effort, expected) in [
        ("pro", "high", "only effort standard"),
        ("xhigh", "standard", "only effort high"),
    ] {
        let output = modern_fake_run(&root, &prompt, &provider, model, effort, false);
        assert_usage(output, expected);
    }
    let output = modern_fake_run(&root, &prompt, &provider, "pro", "standard", true);
    assert_usage(output, "forbids Docker options and live-send gates");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn numeric_identifier_and_timeout_bounds_fail_before_service_side_effects() {
    assert_usage(
        run([
            "allocate",
            "--json",
            "--request-id",
            "request-ok",
            "--run-id",
            "run-ok",
            "--fencing-token",
            "fence",
            "--ttl-ms",
            "299999",
        ]),
        "compatibility literal 300000",
    );
    assert_usage(
        run([
            "allocate",
            "--json",
            "--request-id",
            "bad/request",
            "--run-id",
            "run-ok",
            "--fencing-token",
            "fence",
        ]),
        "invalid --request-id",
    );

    let root = temp_root("timeout-bounds");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let provider = write_executable(&root, "provider.sh", "#!/usr/bin/env bash\nexit 9\n");
    let prompt = root.join("prompt.txt");
    fs::write(&prompt, "prompt").expect("prompt");
    let run_timeout = Command::new(binary())
        .args([
            "run",
            "--json",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args([
            "--request-id",
            "request-run",
            "--run-id",
            "run-run",
            "--fencing-token",
            "fence",
            "--model",
            "pro",
            "--prompt-file",
        ])
        .arg(&prompt)
        .args([
            "--artifact-expectation",
            "optional",
            "--provider-timeout-ms",
            "499999",
        ])
        .output()
        .expect("run timeout");
    assert_usage(run_timeout, "at least 500000");

    let show = Command::new(binary())
        .args([
            "show",
            "--json",
            "--session",
            "session-ok",
            "--fencing-token",
            "fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--provider-timeout-ms", "199999"])
        .output()
        .expect("show");
    assert_usage(show, "at least 200000");

    let resume = Command::new(binary())
        .args([
            "resume",
            "--json",
            "--session",
            "session-ok",
            "--fencing-token",
            "fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--poll-timeout-seconds", "0"])
        .output()
        .expect("resume");
    assert_usage(resume, "expected 1..=10800");

    let resume_timeout = Command::new(binary())
        .args([
            "resume",
            "--json",
            "--session",
            "session-ok",
            "--fencing-token",
            "fence",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args(["--provider-timeout-ms", "499999"])
        .output()
        .expect("resume timeout");
    assert_usage(resume_timeout, "at least 500000");

    let download = Command::new(binary())
        .args([
            "download",
            "--json",
            "--session",
            "session-ok",
            "--fencing-token",
            "fence",
            "--artifact-expectation",
            "optional",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(&provider)
        .args([
            "--poll-timeout-seconds",
            "1",
            "--provider-timeout-ms",
            "319999",
        ])
        .output()
        .expect("download");
    assert_usage(download, "at least 320000");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn regular_file_and_executable_path_contracts_reject_symlinks_and_missing_execute_bits() {
    let root = temp_root("path-contracts");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private root");
    let prompt = root.join("prompt.txt");
    fs::write(&prompt, "prompt").expect("prompt");
    let prompt_link = root.join("prompt-link.txt");
    symlink(&prompt, &prompt_link).expect("symlink");
    let provider = write_executable(&root, "provider.sh", "#!/usr/bin/env bash\nexit 9\n");
    assert_usage(
        modern_fake_run(&root, &prompt_link, &provider, "pro", "standard", false),
        "non-symlink regular file",
    );

    let non_executable = root.join("not-executable.sh");
    fs::write(&non_executable, "#!/usr/bin/env bash\nexit 9\n").expect("provider");
    assert_usage(
        modern_fake_run(&root, &prompt, &non_executable, "pro", "standard", false),
        "executable file",
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn constants_are_the_exact_four_lf_terminated_lines() {
    let output = run(["constants"]);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"EX_OK=0\nEX_USAGE=2\nEX_HARD=70\nEX_LOCK=75\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn lifecycle_result_matrix_closes_all_ninety_eight_command_pairs() {
    let rows = [
        ("status", "status.ready status.blocked status.degraded status.state_invalid status.runtime_probe_failed status.lock_contended"),
        ("preflight", "preflight.ready preflight.model_correction_required preflight.login_required preflight.subscription_required preflight.provider_limit preflight.unreachable preflight.schema_drift preflight.no_slot preflight.state_invalid preflight.lock_contended"),
        ("run", "run.running run.terminal_success run.terminal_optional_zero run.queued_pool_busy run.model_failed run.upload_failed run.send_failed run.send_uncertain run.poll_failed run.artifact_required_failed run.output_publish_failed run.slot_readiness_failed run.release_failed run.lock_contended"),
        ("show", "show.running show.terminal show.idle show.unknown_session show.pinned_slot_unavailable show.url_rejected show.content_unavailable show.claim_conflict show.request_binding_missing show.provider_blocked show.release_failed show.lock_contended"),
        ("resume", "resume.running resume.terminal_success resume.terminal_optional_zero resume.unknown_session resume.pinned_slot_unavailable resume.url_rejected resume.content_unavailable resume.claim_conflict resume.output_publish_failed resume.request_binding_missing resume.provider_blocked resume.poll_failed resume.artifact_required_failed resume.release_failed resume.lock_contended"),
        ("download", "download.completed download.optional_zero download.unknown_session download.pinned_slot_unavailable download.url_rejected download.claim_conflict download.content_unavailable download.controls_absent_required download.ambiguous_controls download.event_timeout download.integrity_failed download.provider_blocked download.release_failed download.lock_contended"),
        ("release", "release.allocatable release.cooldown_blocked release.already_released release.stop_skipped_owner_alive release.target_unknown release.fencing_mismatch release.takeover_unproven release.stop_failed release.cleanup_failed release.lock_contended"),
        ("cleanup", "cleanup.plan cleanup.applied cleanup.state_invalid cleanup.unsafe_path cleanup.partial_failure cleanup.lock_contended"),
        ("state-rebuild", "state_rebuild.match state_rebuild.head_stale state_rebuild.snapshot_ignored state_rebuild.event_invalid state_rebuild.transition_invalid state_rebuild.digest_mismatch state_rebuild.lock_contended"),
        ("allocate", "allocate.dry_run_candidate allocate.pool_busy allocate.state_invalid allocate.lock_contended"),
    ];
    let nonterminal = "run.running run.queued_pool_busy show.running show.idle resume.running";
    let terminal_success = "status.ready status.blocked status.degraded preflight.ready preflight.model_correction_required run.terminal_success run.terminal_optional_zero show.terminal resume.terminal_success resume.terminal_optional_zero download.completed download.optional_zero release.allocatable release.cooldown_blocked release.already_released release.stop_skipped_owner_alive cleanup.plan cleanup.applied state_rebuild.match state_rebuild.head_stale state_rebuild.snapshot_ignored allocate.dry_run_candidate allocate.pool_busy";
    let recoverable_with_reason =
        "preflight.login_required preflight.subscription_required preflight.provider_limit";
    let mut count = 0;
    for (command, kinds) in rows {
        for result_kind in kinds.split_ascii_whitespace() {
            count += 1;
            let expected = if result_kind.ends_with(".lock_contended") {
                ResultSpec {
                    exit_code: 75,
                    ok: false,
                    reason_required: true,
                    terminal: true,
                }
            } else if nonterminal
                .split_ascii_whitespace()
                .any(|value| value == result_kind)
            {
                ResultSpec {
                    exit_code: 0,
                    ok: true,
                    reason_required: false,
                    terminal: false,
                }
            } else if terminal_success
                .split_ascii_whitespace()
                .any(|value| value == result_kind)
            {
                ResultSpec {
                    exit_code: 0,
                    ok: true,
                    reason_required: false,
                    terminal: true,
                }
            } else if recoverable_with_reason
                .split_ascii_whitespace()
                .any(|value| value == result_kind)
            {
                ResultSpec {
                    exit_code: 0,
                    ok: true,
                    reason_required: true,
                    terminal: true,
                }
            } else {
                ResultSpec {
                    exit_code: 70,
                    ok: false,
                    reason_required: true,
                    terminal: true,
                }
            };
            assert_eq!(result_spec(command, result_kind), Some(expected));
        }
    }
    assert_eq!(count, 98);
    assert_eq!(result_spec("run", "show.running"), None);
    assert_eq!(result_spec("constants", "constants.ready"), None);

    let mut envelope = LifecycleEnvelope::base("run", "operation-1");
    let spec = envelope.select_matrix("run.running").expect("matrix row");
    assert_eq!(spec.exit_code, 0);
    assert_eq!(envelope.result_kind, "run.running");
    assert_eq!(envelope.status, "run.running");
    assert!(envelope.ok);
    assert!(!envelope.terminal);
    assert_eq!(envelope.select_matrix("show.running"), None);
}

fn run<const N: usize>(args: [&str; N]) -> std::process::Output {
    Command::new(binary()).args(args).output().expect("run cli")
}

fn run_os<const N: usize>(args: [OsString; N]) -> std::process::Output {
    Command::new(binary()).args(args).output().expect("run cli")
}

fn assert_usage(output: std::process::Output, message: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "stderr={} expected={message}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn modern_fake_run(
    root: &std::path::Path,
    prompt: &std::path::Path,
    provider: &std::path::Path,
    model: &str,
    effort: &str,
    live_gates: bool,
) -> std::process::Output {
    let mut command = Command::new(binary());
    command
        .env("GPT_WEBAI_STATE_ROOT", root)
        .args([
            "run",
            "--json",
            "--fake-runtime",
            "--fake-provider",
            "--provider-bin",
        ])
        .arg(provider)
        .args(["--request-id", "request-run", "--run-id", "run-run"])
        .args(["--fencing-token", "fixture-fence", "--model", model])
        .args(["--effort", effort, "--prompt-file"])
        .arg(prompt)
        .args(["--artifact-expectation", "optional"]);
    if live_gates {
        command.args(["--live-send", "--require-visual-gate"]);
    }
    command.output().expect("modern run")
}

fn write_preflight_provider(_root: &std::path::Path, _log: &std::path::Path) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider")
        .canonicalize()
        .expect("canonical R13 fake provider")
}

fn write_executable(root: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, body).expect("write executable");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).expect("chmod");
    path
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("gpt-webai-{name}-{}-{nonce}", std::process::id()))
}
