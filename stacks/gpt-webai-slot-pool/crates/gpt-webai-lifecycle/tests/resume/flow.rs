use std::fs;

use gpt_webai_lifecycle::locks::acquire_slot_lease;
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::records::{holder_count, lock_count};
use gpt_webai_lifecycle::resume::resume_session;
use gpt_webai_lifecycle::sessions::read_session_record;
use serde_json::json;

use super::fixtures::Fixture;

#[test]
fn fake_resume_uses_pinned_session_slot_without_locking_or_allocating() {
    let fixture = Fixture::new("done");
    fixture.write_session("sid-resume", "slot-06", "cohort-b");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("sid-resume"));

    let output = resume_session(fixture.input(provider, "sid-resume"));

    assert!(output.ok);
    assert_eq!(output.status, "done");
    assert_eq!(output.session_id.as_deref(), Some("sid-resume"));
    assert_eq!(output.slot_id.as_deref(), Some("slot-06"));
    assert_eq!(output.account_group.as_deref(), Some("cohort-b"));
    assert_eq!(output.provider_status.as_deref(), Some("done"));
    assert_eq!(output.answer_text_len, Some("final answer".len()));
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
    assert_eq!(
        fs::read_to_string(args_file).expect("args"),
        "sessions\nresume\n--session\nsid-resume\n"
    );
}

#[test]
fn fake_resume_persists_answer_artifacts_marks_released_and_clears_stale_lease() {
    let fixture = Fixture::new("done-artifacts-release");
    fixture.write_session("sid-resume-artifacts", "slot-06", "cohort-b");
    acquire_slot_lease(
        &fixture.root,
        "slot-06",
        "request-sid-resume-artifacts",
        "run-sid-resume-artifacts",
        "stale-token",
        0,
    )
    .expect("stale lease");
    assert_eq!(holder_count(&fixture.root), 1);
    assert_eq!(lock_count(&fixture.root), 1);
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("sid-resume-artifacts"));

    let output = resume_session(fixture.input(provider, "sid-resume-artifacts"));

    assert!(output.ok);
    assert_eq!(output.status, "done");
    let answer_dir = fixture
        .root
        .join("requests")
        .join("run-sid-resume-artifacts")
        .join("artifacts");
    assert_eq!(
        fs::read_to_string(answer_dir.join("answer.md")).expect("answer markdown"),
        "final answer"
    );
    let answer_json = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(answer_dir.join("answer.json")).expect("answer json"),
    )
    .expect("parse answer json");
    assert_eq!(answer_json["sessionId"], "sid-resume-artifacts");
    assert_eq!(answer_json["answerText"], "final answer");
    assert_eq!(answer_json["answerTextLen"], "final answer".len());
    let released =
        read_session_record(&fixture.root, "sid-resume-artifacts").expect("released session");
    assert_eq!(released.session_id, "sid-resume-artifacts");
    assert_eq!(released.cohort, "cohort-b");
    assert!(released.updated_at_ms >= released.created_at_ms);
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
}

#[test]
fn fake_resume_refuses_active_slot_lease_before_provider() {
    let fixture = Fixture::new("active-lease");
    fixture.write_session("sid-resume-active", "slot-06", "cohort-b");
    acquire_slot_lease(
        &fixture.root,
        "slot-06",
        "request-sid-resume-active",
        "run-sid-resume-active",
        "active-token",
        30_000,
    )
    .expect("active lease");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("sid-resume-active"));

    let output = resume_session(fixture.input(provider, "sid-resume-active"));

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("lock.active"));
    assert!(!args_file.exists());
    assert_eq!(holder_count(&fixture.root), 1);
    assert_eq!(lock_count(&fixture.root), 1);
}

#[test]
fn fake_resume_fails_closed_before_provider_when_session_record_is_missing() {
    let fixture = Fixture::new("missing");
    let args_file = fixture.root.join("args.txt");
    let provider = fixture.write_provider(&args_file, done_resume("missing"));

    let output = resume_session(fixture.input(provider, "missing"));

    assert!(!output.ok);
    assert_eq!(output.reason.as_deref(), Some("session.record_missing"));
    assert_eq!(output.session_id.as_deref(), Some("missing"));
    assert!(!args_file.exists());
    assert_eq!(holder_count(&fixture.root), 0);
    assert_eq!(lock_count(&fixture.root), 0);
}

fn done_resume(session_id: &str) -> String {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-resume",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    })
    .to_string()
}
