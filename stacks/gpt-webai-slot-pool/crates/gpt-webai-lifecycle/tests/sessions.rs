use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::sessions::{
    mark_session_released, new_session_record, read_request_session_record, read_session_record,
    update_session_record, write_session_record, NewSessionRecord, SessionRecordError,
};

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

fn sample_new_session() -> NewSessionRecord {
    NewSessionRecord {
        request_id: Some("request-a".to_string()),
        run_id: Some("run-a".to_string()),
        session_id: "session-a".to_string(),
        conversation_url: "https://chatgpt.com/c/session-a".to_string(),
        slot_id: "slot-06".to_string(),
        cohort: "cohort-b".to_string(),
        page_binding_generation: 1,
    }
}

#[test]
fn session_record_round_trips_by_session_and_request_key() {
    let root = temp_state_root("session-round-trip");
    let record = new_session_record(sample_new_session()).expect("new session record");
    write_session_record(&root, &record).expect("write session");

    assert_eq!(
        read_session_record(&root, "session-a").expect("read by session"),
        record
    );
    assert_eq!(
        read_request_session_record(&root, "request-a").expect("read by request"),
        record
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_record_rejects_root_or_mismatched_conversation_url() {
    let mut input = sample_new_session();
    input.conversation_url = "https://chatgpt.com/".to_string();

    let error = new_session_record(input).expect_err("root url rejected");
    assert!(matches!(
        error,
        SessionRecordError::InvalidConversationUrl(url) if url == "https://chatgpt.com/"
    ));
}

#[test]
fn reading_session_record_rejects_persisted_url_mismatch() {
    let root = temp_state_root("session-read-url-mismatch");
    let mut record = new_session_record(sample_new_session()).expect("new session record");
    record.conversation_url = "https://chatgpt.com/".to_string();
    let sessions_dir = root.join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    fs::set_permissions(&sessions_dir, fs::Permissions::from_mode(0o700))
        .expect("private sessions dir");
    let record_path = sessions_dir.join("session-a.json");
    fs::write(
        &record_path,
        serde_json::to_string(&record).expect("session json"),
    )
    .expect("write corrupt record");
    fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600))
        .expect("private session record");

    let error = read_session_record(&root, "session-a").expect_err("mismatch rejected");

    assert!(matches!(
        error,
        SessionRecordError::InvalidConversationUrl(url) if url == "https://chatgpt.com/"
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_session_create_is_collision_safe_and_update_preserves_closed_identity() {
    let root = temp_state_root("session-update");
    let record = new_session_record(sample_new_session()).expect("new session record");
    write_session_record(&root, &record).expect("create session");
    assert!(matches!(
        write_session_record(&root, &record),
        Err(SessionRecordError::Collision(session_id)) if session_id == "session-a"
    ));

    let released = mark_session_released(record.clone(), Some("answer.done".to_string()));
    update_session_record(&root, &released).expect("update session");
    let reopened = read_session_record(&root, "session-a").expect("reopen updated session");
    assert_eq!(reopened.created_at_ms, record.created_at_ms);
    assert!(reopened.updated_at_ms >= record.updated_at_ms);
    assert_eq!(reopened.session_id, record.session_id);
    assert_eq!(reopened.slot_id, record.slot_id);
    assert_eq!(reopened.cohort, record.cohort);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_session_update_rejects_changes_to_pinned_identity() {
    let root = temp_state_root("session-update-identity");
    let record = new_session_record(sample_new_session()).expect("new session record");
    write_session_record(&root, &record).expect("create session");

    for changed in [
        {
            let mut value = record.clone();
            value.slot_id = "slot-07".to_string();
            value
        },
        {
            let mut value = record.clone();
            value.cohort = "cohort-c".to_string();
            value
        },
        {
            let mut value = record.clone();
            value.request_id = Some("request-b".to_string());
            value
        },
        {
            let mut value = record.clone();
            value.run_id = Some("run-b".to_string());
            value
        },
    ] {
        assert!(matches!(
            update_session_record(&root, &changed),
            Err(SessionRecordError::Invalid(message)) if message == "session update identity"
        ));
    }
    assert_eq!(
        read_session_record(&root, "session-a").expect("unchanged session"),
        record
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_session_read_rejects_symlink_hardlink_and_non_private_file() {
    let root = temp_state_root("session-file-safety");
    let record = new_session_record(sample_new_session()).expect("new session record");
    write_session_record(&root, &record).expect("create session");
    let path = root.join("sessions/session-a.json");
    let canonical = fs::read(&path).expect("canonical bytes");

    fs::remove_file(&path).expect("remove target");
    let outside = root.join("outside.json");
    fs::write(&outside, &canonical).expect("outside bytes");
    symlink(&outside, &path).expect("session symlink");
    assert!(read_session_record(&root, "session-a").is_err());

    fs::remove_file(&path).expect("remove symlink");
    fs::hard_link(&outside, &path).expect("session hardlink");
    assert!(matches!(
        read_session_record(&root, "session-a"),
        Err(SessionRecordError::Invalid(message)) if message == "unsafe session file"
    ));

    fs::remove_file(&path).expect("remove hardlink");
    fs::write(&path, &canonical).expect("restore bytes");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
    assert!(matches!(
        read_session_record(&root, "session-a"),
        Err(SessionRecordError::Invalid(message)) if message == "unsafe session file"
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_session_allows_null_request_binding_but_rejects_unknown_fields() {
    let root = temp_state_root("session-closed");
    let mut input = sample_new_session();
    input.request_id = None;
    input.run_id = None;
    let record = new_session_record(input).expect("requestless session");
    write_session_record(&root, &record).expect("write requestless session");
    assert!(read_session_record(&root, "session-a")
        .expect("read requestless session")
        .request_binding()
        .is_err());

    let path = root.join("sessions/session-a.json");
    let mut value = serde_json::to_value(&record).expect("session value");
    value
        .as_object_mut()
        .expect("session object")
        .insert("legacyStatus".to_string(), serde_json::json!("running"));
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&value).unwrap()),
    )
    .expect("write unknown field");
    assert!(matches!(
        read_session_record(&root, "session-a"),
        Err(SessionRecordError::Json(_))
    ));
    let _ = fs::remove_dir_all(root);
}
