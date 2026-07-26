use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::confirmation::{confirm_send_started, confirm_terminal_answer};
use gpt_webai_lifecycle::contracts::ids::{
    artifact_host_saved_rel_path, derive_artifact_id, derive_download_event_id,
    derive_page_binding_id, derive_session_binding_id, h256, sha256_hex,
};
use gpt_webai_lifecycle::contracts::provider::{ProviderRequest, ProviderResponse};
use gpt_webai_lifecycle::journal::canonical::canonical_bytes;
use gpt_webai_lifecycle::provider_client::{
    run_provider_invocation, run_r13_provider_invocation, ProviderInvocation,
    ProviderInvocationError, ProviderOperation, R13ProviderInvocation, R13ProviderInvocationError,
    PROVIDER_SCHEMA,
};
use gpt_webai_lifecycle::provider_runner::{R13ProviderCommand, R13ProviderPaths};
use gpt_webai_lifecycle::request::artifact_expectation::ArtifactExpectation;
use serde_json::{json, Value};

#[test]
fn r13_request_file_round_trip_reopens_canonical_receipt() {
    let fixture = R13Fixture::new("round-trip", 0, false);
    let result =
        run_r13_provider_invocation(&fixture.invocation()).expect("R13 provider round trip");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.response, fixture.response);
    assert_eq!(result.receipt_ids, vec![result.receipt_id.clone()]);
    assert_eq!(
        fs::read_to_string(&fixture.args_file).expect("args"),
        format!(
            "--request-file\n{}\n",
            fixture.command.paths.request_host_path.display()
        )
    );
    let request_bytes = fs::read(&fixture.command.paths.request_host_path).expect("request bytes");
    assert_eq!(request_bytes, canonical_bytes(&fixture.request).unwrap());
    assert_eq!(result.request_sha256, h256(request_bytes));
    assert_eq!(
        fs::metadata(&fixture.command.paths.request_host_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn r13_rejects_stdout_trailing_bytes_even_with_a_valid_receipt() {
    let fixture = R13Fixture::new("trailing", 0, true);
    let error =
        run_r13_provider_invocation(&fixture.invocation()).expect_err("trailing bytes rejected");

    assert!(matches!(error, R13ProviderInvocationError::Canonical(_)));
}

#[test]
fn r13_rejects_nonzero_rc_with_a_success_envelope() {
    let fixture = R13Fixture::new("rc-mismatch", 70, false);
    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("rc/envelope mismatch rejected");

    assert!(matches!(
        error,
        R13ProviderInvocationError::ExitEnvelopeMismatch { code: 70 }
    ));
}

#[test]
fn r13_rejects_existing_different_provider_request() {
    let fixture = R13Fixture::new("request-collision", 0, false);
    fs::write(
        &fixture.command.paths.request_host_path,
        b"{\"different\":true}\n",
    )
    .expect("collision request");
    fs::set_permissions(
        &fixture.command.paths.request_host_path,
        fs::Permissions::from_mode(0o600),
    )
    .expect("request mode");

    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("different immutable request rejected");
    assert!(matches!(
        error,
        R13ProviderInvocationError::RequestCollision(_)
    ));
}

#[test]
fn r13_rejects_request_fifo_before_spawning_provider() {
    let fixture = R13Fixture::new("request-fifo", 0, false);
    make_fifo(&fixture.command.paths.request_host_path);

    let started = Instant::now();
    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("request FIFO must be rejected without blocking");

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
    assert!(!fixture.args_file.exists(), "provider must not be spawned");
}

#[test]
fn r13_rejects_host_roots_outside_the_configured_state_root() {
    let fixture = R13Fixture::new("outside-state-root", 0, false);
    let unrelated_root = fixture._dir.path().join("unrelated-state");
    fs::create_dir(&unrelated_root).expect("unrelated root");
    fs::set_permissions(&unrelated_root, fs::Permissions::from_mode(0o700))
        .expect("unrelated mode");
    let invocation = R13ProviderInvocation {
        command: &fixture.command,
        request: &fixture.request,
        state_root: &unrelated_root,
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 1_048_576,
        max_stderr_bytes: 262_144,
    };

    let error = run_r13_provider_invocation(&invocation)
        .expect_err("external provider roots must be rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
    assert!(!fixture.args_file.exists(), "provider must not be spawned");
}

#[test]
fn r13_rejects_receipt_bytes_that_do_not_match_the_response_reference() {
    let fixture = R13Fixture::new("receipt-corrupt", 0, false);
    fs::write(&fixture.receipt_template, b"corrupt\n").expect("corrupt receipt template");

    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("corrupt receipt must be rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_rejects_fifo_and_hardlinked_receipts_without_blocking() {
    let mut fifo_fixture = R13Fixture::new("receipt-fifo", 0, false);
    let fifo_receipt = fifo_fixture
        .command
        .paths
        .operation_host_dir
        .join("provider-receipt.json");
    make_fifo(&fifo_receipt);
    fifo_fixture.command.provider_bin =
        write_r13_response_only_provider(fifo_fixture._dir.path(), &fifo_fixture.response);
    let started = Instant::now();
    let error = run_r13_provider_invocation(&fifo_fixture.invocation())
        .expect_err("receipt FIFO must be rejected without blocking");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));

    let mut hardlink_fixture = R13Fixture::new("receipt-hardlink", 0, false);
    let receipt_path = hardlink_fixture
        .command
        .paths
        .operation_host_dir
        .join("provider-receipt.json");
    fs::copy(&hardlink_fixture.receipt_template, &receipt_path).expect("receipt target");
    fs::set_permissions(&receipt_path, fs::Permissions::from_mode(0o600)).expect("receipt mode");
    fs::hard_link(
        &receipt_path,
        hardlink_fixture._dir.path().join("receipt-alias.json"),
    )
    .expect("receipt hard link");
    hardlink_fixture.command.provider_bin =
        write_r13_response_only_provider(hardlink_fixture._dir.path(), &hardlink_fixture.response);
    let error = run_r13_provider_invocation(&hardlink_fixture.invocation())
        .expect_err("hard-linked receipt must be rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_accepts_rc_124_only_for_a_valid_running_poll_envelope() {
    let fixture = R13PollFixture::running("poll-rc124", 124);
    let result =
        run_r13_provider_invocation(&fixture.invocation()).expect("running poll timeout envelope");

    assert_eq!(result.exit_code, 124);
    assert_eq!(result.response.status, "running");
}

#[test]
fn r13_poll_reopens_terminal_answer_beneath_the_artifact_host_root() {
    let fixture = R13PollFixture::terminal("poll-answer-root", b"terminal answer\n");

    run_r13_provider_invocation(&fixture.invocation())
        .expect("answer under artifact host root must reopen");
    assert!(!fixture
        .command
        .paths
        .operation_host_dir
        .join(&fixture.answer_rel_path)
        .exists());
}

#[test]
fn r13_poll_rejects_non_private_or_symlinked_answer_paths() {
    let mode_fixture = R13PollFixture::terminal("poll-answer-mode", b"terminal answer\n");
    fs::set_permissions(
        mode_fixture.answer_path(),
        fs::Permissions::from_mode(0o644),
    )
    .expect("answer mode");
    let error = run_r13_provider_invocation(&mode_fixture.invocation())
        .expect_err("non-private answer rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));

    let symlink_fixture = R13PollFixture::terminal("poll-answer-symlink", b"terminal answer\n");
    let answer_parent = symlink_fixture
        .command
        .paths
        .artifacts_host_dir
        .join("answers");
    let outside = symlink_fixture._dir.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    fs::rename(symlink_fixture.answer_path(), outside.join("final.md"))
        .expect("move answer outside");
    fs::remove_dir(&answer_parent).expect("remove original answer directory");
    std::os::unix::fs::symlink(&outside, &answer_parent).expect("symlink answer directory");
    let error = run_r13_provider_invocation(&symlink_fixture.invocation())
        .expect_err("symlinked answer parent rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));

    let hardlink_fixture = R13PollFixture::terminal("poll-answer-hardlink", b"terminal answer\n");
    fs::hard_link(
        hardlink_fixture.answer_path(),
        hardlink_fixture._dir.path().join("answer-alias.md"),
    )
    .expect("create answer hard link");
    let error = run_r13_provider_invocation(&hardlink_fixture.invocation())
        .expect_err("hard-linked answer rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));

    let fifo_fixture = R13PollFixture::terminal("poll-answer-fifo", b"terminal answer\n");
    fs::remove_file(fifo_fixture.answer_path()).expect("remove answer file");
    make_fifo(&fifo_fixture.answer_path());
    let started = Instant::now();
    let error = run_r13_provider_invocation(&fifo_fixture.invocation())
        .expect_err("answer FIFO must be rejected without blocking");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_artifact_click_save_reopens_regular_download_beneath_artifact_root() {
    let fixture = R13DownloadFixture::new("download-regular", b"zip test\n", None, None);

    run_r13_provider_invocation(&fixture.invocation())
        .expect("regular private download beneath artifact root must reopen");
}

#[test]
fn r13_artifact_click_save_rejects_symlinked_download_parent() {
    let fixture = R13DownloadFixture::new("download-parent-symlink", b"zip test\n", None, None);
    let download_path = fixture.download_path();
    let download_parent = download_path.parent().unwrap();
    let outside = fixture._dir.path().join("outside-downloads");
    fs::create_dir(&outside).expect("outside download directory");
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700))
        .expect("outside directory mode");
    fs::rename(fixture.download_path(), outside.join("result.zip")).expect("move download outside");
    fs::remove_dir(download_parent).expect("remove original download directory");
    std::os::unix::fs::symlink(&outside, download_parent).expect("symlink download directory");

    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("symlinked download parent rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_artifact_click_save_rejects_hardlinked_download() {
    let fixture = R13DownloadFixture::new("download-hardlink", b"zip test\n", None, None);
    fs::hard_link(
        fixture.download_path(),
        fixture._dir.path().join("download-alias.zip"),
    )
    .expect("create download hard link");

    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("hard-linked download rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_artifact_click_save_rejects_download_fifo_without_blocking() {
    let fixture = R13DownloadFixture::new("download-fifo", b"zip test\n", None, None);
    fs::remove_file(fixture.download_path()).expect("remove download file");
    make_fifo(&fixture.download_path());

    let started = Instant::now();
    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("download FIFO must be rejected without blocking");

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_artifact_click_save_rejects_non_private_download() {
    let fixture = R13DownloadFixture::new("download-mode", b"zip test\n", None, None);
    fs::set_permissions(fixture.download_path(), fs::Permissions::from_mode(0o644))
        .expect("download mode");

    let error = run_r13_provider_invocation(&fixture.invocation())
        .expect_err("non-private download rejected");
    assert!(matches!(error, R13ProviderInvocationError::Receipt(_)));
}

#[test]
fn r13_artifact_click_save_rejects_download_digest_or_size_mismatch() {
    let bytes = b"zip test\n";
    let digest_fixture = R13DownloadFixture::new(
        "download-digest-mismatch",
        bytes,
        Some(h256(b"bad test\n")),
        None,
    );
    let error = run_r13_provider_invocation(&digest_fixture.invocation())
        .expect_err("download digest mismatch rejected");
    assert!(matches!(
        error,
        R13ProviderInvocationError::Receipt("download digest/size")
    ));

    let size_fixture = R13DownloadFixture::new(
        "download-size-mismatch",
        bytes,
        None,
        Some(bytes.len() as u64 + 1),
    );
    let error = run_r13_provider_invocation(&size_fixture.invocation())
        .expect_err("download size mismatch rejected");
    assert!(matches!(
        error,
        R13ProviderInvocationError::Receipt("download digest/size")
    ));
}

#[test]
fn runs_fake_send_provider_and_confirms_real_start_evidence() {
    let dir = TestDir::new("send");
    let fake_provider = write_fake_provider(dir.path());
    let prompt_file = dir.path().join("prompt.md");
    fs::write(&prompt_file, "hello").expect("write prompt");
    let uploaded = dir.path().join("canary.txt");
    fs::write(&uploaded, "canary").expect("write upload");
    let args_file = dir.path().join("args.txt");
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "sent",
        "sessionId": "sid-send",
        "targetId": "target-send",
        "conversationUrl": "https://chatgpt.com/c/sid-send",
        "turnEvidence": {
            "activeTurn": true,
            "userTurnId": format!("turn_{}", "1".repeat(64)),
            "assistantTurnId": format!("turn_{}", "2".repeat(64))
        }
    });

    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Send {
            prompt_file: prompt_file.clone(),
            model: "gpt-5-thinking".to_string(),
            effort: "high".to_string(),
            files: vec![uploaded.clone()],
        },
        env: fake_provider_env(&args_file, &stdout.to_string(), 0),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 16_384,
        max_stderr_bytes: 1_024,
    })
    .expect("provider send invocation");

    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.summary.status, "sent");
    assert_eq!(
        fs::read_to_string(args_file).expect("args"),
        format!(
            "send\n--prompt-file\n{}\n--model\ngpt-5-thinking\n--effort\nhigh\n--file\n{}\n",
            prompt_file.display(),
            uploaded.display()
        )
    );
    let confirmation = confirm_send_started(&result.value).expect("confirmed start");
    assert_eq!(confirmation.session_id, "sid-send");
}

#[test]
fn runs_fake_poll_provider_and_confirms_terminal_answer() {
    let dir = TestDir::new("poll");
    let fake_provider = write_fake_provider(dir.path());
    let args_file = dir.path().join("args.txt");
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": "sid-poll",
        "targetId": "target-poll",
        "conversationUrl": "https://chatgpt.com/c/sid-poll",
        "answerText": "final answer",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    });

    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Poll {
            session_id: "sid-poll".to_string(),
            timeout_seconds: 30,
            artifact_expectation: ArtifactExpectation::Optional,
        },
        env: fake_provider_env(&args_file, &stdout.to_string(), 0),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 16_384,
        max_stderr_bytes: 1_024,
    })
    .expect("provider poll invocation");

    assert_eq!(result.summary.status, "done");
    assert_eq!(
        fs::read_to_string(args_file).expect("args"),
        "poll\n--session\nsid-poll\n--timeout\n30\n--artifact-expectation\noptional\n"
    );
    let confirmation = confirm_terminal_answer(&result.value).expect("terminal answer");
    assert_eq!(confirmation.answer_text_len, "final answer".len());
}

#[test]
fn preserves_nonzero_exit_when_provider_returns_valid_failure_envelope() {
    let dir = TestDir::new("nonzero");
    let fake_provider = write_fake_provider(dir.path());
    let args_file = dir.path().join("args.txt");
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": false,
        "vendor": "chatgpt",
        "status": "provider.schema_drift",
        "reason": "missing schema field"
    });

    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Status,
        env: fake_provider_env(&args_file, &stdout.to_string(), 70),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 16_384,
        max_stderr_bytes: 1_024,
    })
    .expect("valid failure envelope");

    assert_eq!(result.exit_code, Some(70));
    assert_eq!(result.summary.status, "provider.schema_drift");
    assert!(!result.summary.ok);
}

#[test]
fn rejects_invalid_provider_stdout_json() {
    let dir = TestDir::new("invalid-json");
    let fake_provider = write_fake_provider(dir.path());
    let args_file = dir.path().join("args.txt");

    let error = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Status,
        env: fake_provider_env(&args_file, "not json", 0),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 16_384,
        max_stderr_bytes: 1_024,
    })
    .expect_err("invalid stdout rejected");

    assert!(matches!(error, ProviderInvocationError::Json(_)));
}

#[test]
fn drains_large_provider_stdout_without_deadlocking_capture() {
    let dir = TestDir::new("large-capture-stdout");
    let fake_provider = write_stdout_file_provider(dir.path());
    let stdout_file = dir.path().join("large-capture.json");
    let filler = "x".repeat(2 * 1024 * 1024);
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "sessionId": "sid-large-capture",
        "conversationUrl": "https://chatgpt.com/c/sid-large-capture",
        "diagnostics": {
            "screenshot": "saved",
            "dom": "saved",
            "bodyTextPreview": filler
        }
    });
    fs::write(&stdout_file, stdout.to_string()).expect("write large stdout");

    let result = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Capture {
            session_id: Some("sid-large-capture".to_string()),
            label: "pre-poll-wait-gate".to_string(),
        },
        env: stdout_file_provider_env(&stdout_file, 0),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 4 * 1024 * 1024,
        max_stderr_bytes: 1_024,
    })
    .expect("large capture stdout should be drained before wait");

    assert_eq!(result.summary.status, "captured");
    assert!(result.stdout_bytes > 2 * 1024 * 1024);
}

#[test]
fn reports_stdout_too_large_after_draining_provider_stdout() {
    let dir = TestDir::new("stdout-too-large");
    let fake_provider = write_stdout_file_provider(dir.path());
    let stdout_file = dir.path().join("too-large-capture.json");
    let filler = "x".repeat(2 * 1024 * 1024);
    let stdout = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "sessionId": "sid-too-large",
        "conversationUrl": "https://chatgpt.com/c/sid-too-large",
        "diagnostics": { "bodyTextPreview": filler }
    });
    fs::write(&stdout_file, stdout.to_string()).expect("write oversized stdout");

    let error = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Capture {
            session_id: Some("sid-too-large".to_string()),
            label: "pre-poll-wait-gate".to_string(),
        },
        env: stdout_file_provider_env(&stdout_file, 0),
        timeout: Duration::from_secs(2),
        max_stdout_bytes: 1_024,
        max_stderr_bytes: 1_024,
    })
    .expect_err("oversized stdout rejected after drain");

    assert!(matches!(
        error,
        ProviderInvocationError::StdoutTooLarge { actual, .. } if actual > 2 * 1024 * 1024
    ));
}

#[test]
fn times_out_slow_provider_process() {
    let dir = TestDir::new("timeout");
    let fake_provider = write_sleeping_provider(dir.path());

    let error = run_provider_invocation(&ProviderInvocation {
        provider_bin: fake_provider,
        args_prefix: Vec::new(),
        operation: ProviderOperation::Status,
        env: Vec::new(),
        timeout: Duration::from_millis(50),
        max_stdout_bytes: 16_384,
        max_stderr_bytes: 1_024,
    })
    .expect_err("slow provider timed out");

    assert!(matches!(error, ProviderInvocationError::Timeout(_)));
}

struct R13Fixture {
    _dir: TestDir,
    request: ProviderRequest,
    response: ProviderResponse,
    command: R13ProviderCommand,
    args_file: PathBuf,
    receipt_template: PathBuf,
}

impl R13Fixture {
    fn new(name: &str, exit_code: i32, trailing_bytes: bool) -> Self {
        let dir = TestDir::new(name);
        let operation_root = dir.path().join("evidence/diagnostics/operation-1");
        let artifacts_root = dir.path().join("artifacts/d-operation-1");
        fs::create_dir_all(&operation_root).expect("operation root");
        fs::create_dir_all(&artifacts_root).expect("artifacts root");
        fs::set_permissions(&operation_root, fs::Permissions::from_mode(0o700))
            .expect("operation mode");
        fs::set_permissions(&artifacts_root, fs::Permissions::from_mode(0o700))
            .expect("artifact mode");

        let operation_data = json!({
            "healthStatus":"ready",
            "dockerStatus":"running",
            "retryAfterMs":null,
            "modelLabel":"pro",
            "composerReady":true
        });
        let request: ProviderRequest = serde_json::from_value(json!({
            "deadlineMs":20_000,
            "evidence":{
                "cdpRelPath":"cdp.sanitized.json",
                "domRelPath":"dom.sanitized.json",
                "receiptRelPaths":{
                    "primary":"provider-receipt.json",
                    "preClick":null,
                    "postClick":null,
                    "reconcile":null
                },
                "screenshotRelPath":"screenshot.privacy-crop.png"
            },
            "identity":{
                "cohort":"cohort-a",
                "operationId":"operation-1",
                "requestId":null,
                "runId":null,
                "sessionId":null,
                "slotId":"slot-01"
            },
            "operation":"status",
            "operationData":{"expectedSlotId":"slot-01","probeAttempt":0},
            "schema":"gpt-webai.provider.request.r13.v1"
        }))
        .expect("request");

        let mut receipt = json!({
            "createdAtMs":1,
            "operation":"status",
            "operationId":"operation-1",
            "payload":operation_data,
            "receiptId":"",
            "requestId":null,
            "runId":null,
            "schema":"pr72.receipt.r13.v1",
            "sessionId":null
        });
        let receipt_preimage = canonical_bytes(&receipt).expect("receipt preimage");
        receipt["receiptId"] = json!(format!("receipt_{}", sha256_hex(receipt_preimage)));
        let receipt_bytes = canonical_bytes(&receipt).expect("receipt bytes");
        let receipt_template = dir.path().join("receipt.json");
        fs::write(&receipt_template, &receipt_bytes).expect("receipt template");

        let response: ProviderResponse = serde_json::from_value(json!({
            "identity":request.identity.clone(),
            "ok":true,
            "operation":"status",
            "operationData":receipt["payload"],
            "providerReason":null,
            "receipt":{
                "path":"provider-receipt.json",
                "sha256":h256(&receipt_bytes),
                "sizeBytes":receipt_bytes.len(),
                "mediaType":"application/json"
            },
            "schema":"gpt-webai.provider.response.r13.v1",
            "status":"done"
        }))
        .expect("response");
        let mut response_bytes = canonical_bytes(&response).expect("response bytes");
        if trailing_bytes {
            response_bytes.extend_from_slice(b"trailing");
        }
        let response_file = dir.path().join("response.json");
        fs::write(&response_file, response_bytes).expect("response file");
        let args_file = dir.path().join("r13-args.txt");
        let provider = dir.path().join("r13-provider.sh");
        fs::write(
            &provider,
            format!(
                "#!/bin/bash\nprintf '%s\\n' \"$@\" > '{}'\nrequest_file=\"$2\"\noperation_root=\"${{request_file%/*}}\"\n/bin/cp '{}' \"$operation_root/provider-receipt.json\"\n/bin/chmod 600 \"$operation_root/provider-receipt.json\"\n/bin/cat '{}'\nexit {}\n",
                args_file.display(),
                receipt_template.display(),
                response_file.display(),
                exit_code
            ),
        )
        .expect("provider script");
        set_executable(&provider);

        let request_path = operation_root.join("provider-request.json");
        Self {
            _dir: dir,
            request,
            response,
            command: R13ProviderCommand {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
                slot_id: "slot-01".to_string(),
                request_key: "d-operation-1".to_string(),
                operation_id: "operation-1".to_string(),
                paths: R13ProviderPaths {
                    operation_host_dir: operation_root.clone(),
                    operation_container_dir: operation_root.clone(),
                    request_host_path: request_path.clone(),
                    request_container_path: request_path,
                    artifacts_host_dir: artifacts_root.clone(),
                    artifacts_container_dir: artifacts_root,
                },
            },
            args_file,
            receipt_template,
        }
    }

    fn invocation(&self) -> R13ProviderInvocation<'_> {
        R13ProviderInvocation {
            command: &self.command,
            request: &self.request,
            state_root: self._dir.path(),
            timeout: Duration::from_secs(2),
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 262_144,
        }
    }
}

struct R13PollFixture {
    _dir: TestDir,
    request: ProviderRequest,
    command: R13ProviderCommand,
    answer_rel_path: PathBuf,
}

impl R13PollFixture {
    fn running(name: &str, exit_code: i32) -> Self {
        Self::new(name, exit_code, None)
    }

    fn terminal(name: &str, answer: &[u8]) -> Self {
        Self::new(name, 0, Some(answer))
    }

    fn new(name: &str, exit_code: i32, terminal_answer: Option<&[u8]>) -> Self {
        let dir = TestDir::new(name);
        let operation_root = dir
            .path()
            .join("evidence/requests/r-request-1/operations/poll-1");
        let artifacts_root = dir.path().join("artifacts/r-request-1");
        fs::create_dir_all(&operation_root).expect("operation root");
        fs::create_dir_all(&artifacts_root).expect("artifacts root");
        fs::set_permissions(&operation_root, fs::Permissions::from_mode(0o700))
            .expect("operation mode");
        fs::set_permissions(&artifacts_root, fs::Permissions::from_mode(0o700))
            .expect("artifacts mode");

        let expected = r13_session_echo(false, None);
        let request: ProviderRequest = serde_json::from_value(json!({
            "deadlineMs":500_000,
            "evidence":{
                "cdpRelPath":"cdp.sanitized.json",
                "domRelPath":"dom.sanitized.json",
                "receiptRelPaths":{
                    "primary":"provider-receipt.json",
                    "preClick":null,
                    "postClick":null,
                    "reconcile":null
                },
                "screenshotRelPath":"screenshot.privacy-crop.png"
            },
            "identity":{
                "cohort":"cohort-a",
                "operationId":"poll-1",
                "requestId":"request-1",
                "runId":"run-1",
                "sessionId":"session_1",
                "slotId":"slot-01"
            },
            "operation":"poll",
            "operationData":{
                "expected":expected,
                "pollAttemptId":"poll-1",
                "pollTimeoutSeconds":300,
                "artifactExpectation":"none"
            },
            "schema":"gpt-webai.provider.request.r13.v1"
        }))
        .expect("poll request");

        let answer_rel_path = PathBuf::from("answers/final.md");
        let operation_data = if let Some(answer) = terminal_answer {
            let answer_path = artifacts_root.join(&answer_rel_path);
            fs::create_dir_all(answer_path.parent().expect("answer parent"))
                .expect("answer parent");
            fs::write(&answer_path, answer).expect("answer bytes");
            fs::set_permissions(&answer_path, fs::Permissions::from_mode(0o600))
                .expect("answer mode");
            json!({
                "expected":r13_session_echo(false, None),
                "observedEcho":r13_session_echo(false, Some(h256(answer))),
                "pollState":"terminal",
                "answerSha256":h256(answer),
                "answerSizeBytes":answer.len(),
                "answerRelPath":answer_rel_path.to_string_lossy(),
                "terminalAssistantTurnId":r13_prefixed("turn", 'b'),
                "bottomProof":null
            })
        } else {
            json!({
                "expected":r13_session_echo(false, None),
                "observedEcho":r13_session_echo(false, None),
                "pollState":"running",
                "answerSha256":null,
                "answerSizeBytes":null,
                "answerRelPath":null,
                "terminalAssistantTurnId":null,
                "bottomProof":null
            })
        };
        let receipt_bytes = r13_receipt_bytes(&request, &operation_data);
        let receipt_template = dir.path().join("poll-receipt.json");
        fs::write(&receipt_template, &receipt_bytes).expect("receipt template");
        let response: ProviderResponse = serde_json::from_value(json!({
            "identity":request.identity.clone(),
            "ok":true,
            "operation":"poll",
            "operationData":operation_data,
            "providerReason":null,
            "receipt":{
                "path":"provider-receipt.json",
                "sha256":h256(&receipt_bytes),
                "sizeBytes":receipt_bytes.len(),
                "mediaType":"application/json"
            },
            "schema":"gpt-webai.provider.response.r13.v1",
            "status":if terminal_answer.is_some(){"done"}else{"running"}
        }))
        .expect("poll response");
        let response_file = dir.path().join("poll-response.json");
        fs::write(
            &response_file,
            canonical_bytes(&response).expect("response bytes"),
        )
        .expect("response file");
        let provider =
            write_r13_fixture_provider(dir.path(), &receipt_template, &response_file, exit_code);
        let request_path = operation_root.join("provider-request.json");
        Self {
            _dir: dir,
            request,
            command: R13ProviderCommand {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
                slot_id: "slot-01".to_string(),
                request_key: "r-request-1".to_string(),
                operation_id: "poll-1".to_string(),
                paths: R13ProviderPaths {
                    operation_host_dir: operation_root.clone(),
                    operation_container_dir: operation_root.clone(),
                    request_host_path: request_path.clone(),
                    request_container_path: request_path,
                    artifacts_host_dir: artifacts_root.clone(),
                    artifacts_container_dir: artifacts_root,
                },
            },
            answer_rel_path,
        }
    }

    fn answer_path(&self) -> PathBuf {
        self.command
            .paths
            .artifacts_host_dir
            .join(&self.answer_rel_path)
    }

    fn invocation(&self) -> R13ProviderInvocation<'_> {
        R13ProviderInvocation {
            command: &self.command,
            request: &self.request,
            state_root: self._dir.path(),
            timeout: Duration::from_secs(2),
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 262_144,
        }
    }
}

struct R13DownloadFixture {
    _dir: TestDir,
    request: ProviderRequest,
    command: R13ProviderCommand,
    download_rel_path: PathBuf,
}

impl R13DownloadFixture {
    fn new(
        name: &str,
        download_bytes: &[u8],
        claimed_sha256: Option<String>,
        claimed_size_bytes: Option<u64>,
    ) -> Self {
        let dir = TestDir::new(name);
        let operation_root = dir
            .path()
            .join("evidence/requests/r-request-1/operations/operation-1");
        let artifacts_root = dir.path().join("artifacts/r-request-1");
        fs::create_dir_all(&operation_root).expect("operation root");
        fs::create_dir_all(&artifacts_root).expect("artifacts root");
        fs::set_permissions(&operation_root, fs::Permissions::from_mode(0o700))
            .expect("operation mode");
        fs::set_permissions(&artifacts_root, fs::Permissions::from_mode(0o700))
            .expect("artifacts mode");

        let artifact_claim_id = r13_prefixed("artifact_claim", '5');
        let page_incarnation_id = r13_prefixed("page", '4');
        let control = r13_artifact_control();
        let control_id = control["controlId"].as_str().expect("controlId");
        let download_event_id =
            derive_download_event_id(&page_incarnation_id, "fixture-download-guid", "result.zip")
                .expect("download event id");
        let artifact_id = derive_artifact_id(&artifact_claim_id, control_id, &download_event_id)
            .expect("artifact id");
        let download_rel_path = PathBuf::from(
            artifact_host_saved_rel_path("r-request-1", &artifact_claim_id, &artifact_id)
                .expect("download relative path"),
        );
        let download_path = dir.path().join(&download_rel_path);
        fs::create_dir(download_path.parent().expect("download parent")).expect("download parent");
        fs::set_permissions(
            download_path.parent().expect("download parent"),
            fs::Permissions::from_mode(0o700),
        )
        .expect("download parent mode");
        fs::write(&download_path, download_bytes).expect("download bytes");
        fs::set_permissions(&download_path, fs::Permissions::from_mode(0o600))
            .expect("download mode");

        let expected = r13_session_echo(false, Some(h256(b"terminal answer\n")));
        let request: ProviderRequest = serde_json::from_value(json!({
            "deadlineMs":320_000,
            "evidence":{
                "cdpRelPath":"cdp.sanitized.json",
                "domRelPath":"dom.sanitized.json",
                "receiptRelPaths":{
                    "primary":"provider-receipt.json",
                    "preClick":null,
                    "postClick":null,
                    "reconcile":null
                },
                "screenshotRelPath":"screenshot.privacy-crop.png"
            },
            "identity":{
                "cohort":"cohort-a",
                "operationId":"operation-1",
                "requestId":"request-1",
                "runId":"run-1",
                "sessionId":"session_1",
                "slotId":"slot-01"
            },
            "operation":"artifact-click-save",
            "operationData":{
                "expected":expected,
                "artifactClaimId":artifact_claim_id,
                "terminalAssistantTurnId":r13_prefixed("turn", 'b'),
                "control":control,
                "baseline":{
                    "baselineSha256":h256(b"baseline"),
                    "capturedAtMs":1,
                    "directory":download_path.parent().expect("download directory")
                        .strip_prefix(dir.path()).expect("state-root relative directory")
                        .to_string_lossy(),
                    "entries":[]
                },
                "controlIndex":0,
                "hostSaveDirectory":download_path.parent().expect("download directory")
                    .strip_prefix(dir.path()).expect("state-root relative directory")
                    .to_string_lossy()
            },
            "schema":"gpt-webai.provider.request.r13.v1"
        }))
        .expect("download request");

        let operation_data = json!({
            "downloadReceipt":{
                "artifactClaimId":artifact_claim_id,
                "artifactId":artifact_id,
                "browserContextId":r13_prefixed("ctx", '2'),
                "clickedAtMs":2,
                "control":r13_artifact_control(),
                "conversationUrl":"https://chatgpt.com/c/session_1",
                "downloadEventId":download_event_id,
                "hostSavedRelPath":download_rel_path.to_string_lossy(),
                "listenerArmedAtMs":1,
                "mediaType":"application/octet-stream",
                "pageIncarnationId":page_incarnation_id,
                "receivedAtMs":3,
                "sessionId":"session_1",
                "sha256":claimed_sha256.unwrap_or_else(|| h256(download_bytes)),
                "sizeBytes":claimed_size_bytes.unwrap_or(download_bytes.len() as u64),
                "slotId":"slot-01",
                "targetId":r13_prefixed("target", '8'),
                "terminalAssistantTurnId":r13_prefixed("turn", 'b')
            },
            "failureReason":null,
            "observedEcho":r13_session_echo(false, Some(h256(b"terminal answer\n")))
        });
        let receipt_bytes = r13_receipt_bytes(&request, &operation_data);
        let receipt_template = dir.path().join("download-receipt.json");
        fs::write(&receipt_template, &receipt_bytes).expect("receipt template");
        let response: ProviderResponse = serde_json::from_value(json!({
            "identity":request.identity.clone(),
            "ok":true,
            "operation":"artifact-click-save",
            "operationData":operation_data,
            "providerReason":null,
            "receipt":{
                "path":"provider-receipt.json",
                "sha256":h256(&receipt_bytes),
                "sizeBytes":receipt_bytes.len(),
                "mediaType":"application/json"
            },
            "schema":"gpt-webai.provider.response.r13.v1",
            "status":"done"
        }))
        .expect("download response");
        let response_file = dir.path().join("download-response.json");
        fs::write(
            &response_file,
            canonical_bytes(&response).expect("response bytes"),
        )
        .expect("response file");
        let provider = write_r13_fixture_provider(dir.path(), &receipt_template, &response_file, 0);
        let request_path = operation_root.join("provider-request.json");
        Self {
            _dir: dir,
            request,
            command: R13ProviderCommand {
                provider_bin: provider,
                args_prefix: Vec::new(),
                env: Vec::new(),
                slot_id: "slot-01".to_string(),
                request_key: "r-request-1".to_string(),
                operation_id: "operation-1".to_string(),
                paths: R13ProviderPaths {
                    operation_host_dir: operation_root.clone(),
                    operation_container_dir: operation_root.clone(),
                    request_host_path: request_path.clone(),
                    request_container_path: request_path,
                    artifacts_host_dir: artifacts_root.clone(),
                    artifacts_container_dir: artifacts_root,
                },
            },
            download_rel_path,
        }
    }

    fn download_path(&self) -> PathBuf {
        self._dir.path().join(&self.download_rel_path)
    }

    fn invocation(&self) -> R13ProviderInvocation<'_> {
        R13ProviderInvocation {
            command: &self.command,
            request: &self.request,
            state_root: self._dir.path(),
            timeout: Duration::from_secs(2),
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 262_144,
        }
    }
}

fn r13_receipt_bytes(request: &ProviderRequest, payload: &Value) -> Vec<u8> {
    let mut receipt = json!({
        "createdAtMs":1,
        "operation":request.operation,
        "operationId":request.identity.operation_id,
        "payload":payload,
        "receiptId":"",
        "requestId":request.identity.request_id,
        "runId":request.identity.run_id,
        "schema":"pr72.receipt.r13.v1",
        "sessionId":request.identity.session_id
    });
    let preimage = canonical_bytes(&receipt).expect("receipt preimage");
    receipt["receiptId"] = json!(format!("receipt_{}", sha256_hex(preimage)));
    canonical_bytes(&receipt).expect("receipt bytes")
}

fn r13_session_echo(active: bool, terminal_answer_sha256: Option<String>) -> Value {
    let page_incarnation_id = r13_prefixed("page", '4');
    let root_binding_hash = format!("sha256:{}", "5".repeat(64));
    json!({
        "activeTurn":active,
        "bindingId":derive_page_binding_id(&page_incarnation_id,&root_binding_hash).unwrap(),
        "bindingGeneration":1,
        "browserContextId":r13_prefixed("ctx", '2'),
        "cohort":"cohort-a",
        "conversationUrl":"https://chatgpt.com/c/session_1",
        "domMutationGeneration":0,
        "leaseGeneration":1,
        "leaseId":r13_prefixed("lease", '3'),
        "pageBindingGeneration":1,
        "pageIncarnationId":page_incarnation_id,
        "requestId":"request-1",
        "rootBindingHash":root_binding_hash,
        "runId":"run-1",
        "runtimeIncarnationId":r13_prefixed("runtime", '6'),
        "runtimeOwnerGeneration":1,
        "runtimeOwnerId":r13_prefixed("owner", '7'),
        "sessionBindingId":derive_session_binding_id("session_1","slot-01","cohort-a").unwrap(),
        "sessionId":"session_1",
        "slotId":"slot-01",
        "targetId":r13_prefixed("target", '8'),
        "terminalAnswerSha256":terminal_answer_sha256,
        "visibleAssistantTurnId":r13_prefixed("turn", 'b'),
        "visibleUserTurnId":r13_prefixed("turn", 'c')
    })
}

fn r13_artifact_control() -> Value {
    json!({
        "boundingBoxHash":h256(b"artifact bounding box"),
        "controlId":r13_prefixed("control", '2'),
        "currentTurnId":r13_prefixed("turn", 'b'),
        "disabled":false,
        "domPathHash":h256(b"artifact dom path"),
        "role":"button",
        "visible":true,
        "visibleTextHash":h256(b"artifact visible text")
    })
}

fn r13_prefixed(prefix: &str, value: char) -> String {
    format!("{prefix}_{}", value.to_string().repeat(64))
}

fn write_r13_fixture_provider(
    dir: &Path,
    receipt_template: &Path,
    response_file: &Path,
    exit_code: i32,
) -> PathBuf {
    let provider = dir.join("r13-frame-provider.sh");
    fs::write(
        &provider,
        format!(
            "#!/bin/bash\nrequest_file=\"$2\"\noperation_root=\"${{request_file%/*}}\"\n/bin/cp '{}' \"$operation_root/provider-receipt.json\"\n/bin/chmod 600 \"$operation_root/provider-receipt.json\"\n/bin/cat '{}'\nexit {}\n",
            receipt_template.display(),
            response_file.display(),
            exit_code
        ),
    )
    .expect("provider script");
    set_executable(&provider);
    provider
}

fn write_r13_response_only_provider(dir: &Path, response: &ProviderResponse) -> PathBuf {
    let response_file = dir.join("response-only.json");
    fs::write(
        &response_file,
        canonical_bytes(response).expect("response bytes"),
    )
    .expect("response file");
    let provider = dir.join("r13-response-only-provider.sh");
    fs::write(
        &provider,
        format!("#!/bin/bash\n/bin/cat '{}'\n", response_file.display()),
    )
    .expect("provider script");
    set_executable(&provider);
    provider
}

fn make_fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).expect("FIFO path");
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
}

fn fake_provider_env(args_file: &Path, stdout: &str, exit_code: i32) -> Vec<(String, String)> {
    vec![
        (
            "FAKE_PROVIDER_ARGS_FILE".to_string(),
            args_file.display().to_string(),
        ),
        ("FAKE_PROVIDER_STDOUT".to_string(), stdout.to_string()),
        ("FAKE_PROVIDER_EXIT".to_string(), exit_code.to_string()),
    ]
}

fn stdout_file_provider_env(stdout_file: &Path, exit_code: i32) -> Vec<(String, String)> {
    vec![
        (
            "FAKE_PROVIDER_STDOUT_FILE".to_string(),
            stdout_file.display().to_string(),
        ),
        ("FAKE_PROVIDER_EXIT".to_string(), exit_code.to_string()),
    ]
}

fn write_fake_provider(dir: &Path) -> PathBuf {
    let path = dir.join("fake-provider.sh");
    fs::write(
        &path,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$FAKE_PROVIDER_ARGS_FILE\"\nprintf '%s\\n' \"$FAKE_PROVIDER_STDOUT\"\nexit \"${FAKE_PROVIDER_EXIT:-0}\"\n",
    )
    .expect("write fake provider");
    set_executable(&path);
    path
}

fn write_stdout_file_provider(dir: &Path) -> PathBuf {
    let path = dir.join("stdout-file-provider.sh");
    fs::write(
        &path,
        "#!/usr/bin/env bash\ncat \"$FAKE_PROVIDER_STDOUT_FILE\"\nprintf '\\n'\nexit \"${FAKE_PROVIDER_EXIT:-0}\"\n",
    )
    .expect("write stdout-file provider");
    set_executable(&path);
    path
}

fn write_sleeping_provider(dir: &Path) -> PathBuf {
    let path = dir.join("sleeping-provider.sh");
    fs::write(&path, "#!/usr/bin/env bash\nsleep 5\n").expect("write sleeping provider");
    set_executable(&path);
    path
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gpt-webai-provider-invocation-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("temp dir mode");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
