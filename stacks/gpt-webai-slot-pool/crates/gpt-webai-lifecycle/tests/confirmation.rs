use gpt_webai_lifecycle::confirmation::{
    confirm_send_started, confirm_terminal_answer, ConfirmationError,
};
use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use serde_json::json;

fn sent_envelope(turn_evidence: serde_json::Value) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "sent",
        "sessionId": "sid-confirm",
        "targetId": "target-abc",
        "conversationUrl": "https://chatgpt.com/c/sid-confirm",
        "turnEvidence": turn_evidence
    })
}

fn proven_turns(active_turn: bool) -> serde_json::Value {
    json!({
        "activeTurn": active_turn,
        "userTurnId": format!("turn_{}", "1".repeat(64)),
        "assistantTurnId": format!("turn_{}", "2".repeat(64))
    })
}

fn done_envelope(answer_text: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": "sid-confirm",
        "targetId": "target-abc",
        "conversationUrl": "https://chatgpt.com/c/sid-confirm",
        "answerText": answer_text,
        "assistantTurn": {
            "turnIndex": 2,
            "domId": "turn-2",
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "turnEvidence": {
            "userCount": 1,
            "assistantCount": 1
        }
    })
}

#[test]
fn confirms_send_only_with_session_url_target_and_real_turn_evidence() {
    let confirmation =
        confirm_send_started(&sent_envelope(proven_turns(true))).expect("confirmed send");

    assert_eq!(confirmation.session_id, "sid-confirm");
    assert_eq!(
        confirmation.conversation_url,
        "https://chatgpt.com/c/sid-confirm"
    );
    assert_eq!(confirmation.target_id, "target-abc");
    assert!(confirmation.active_turn);
    assert_eq!(
        confirmation.user_turn_id,
        format!("turn_{}", "1".repeat(64))
    );
    assert_eq!(
        confirmation.assistant_turn_id,
        format!("turn_{}", "2".repeat(64))
    );
}

#[test]
fn rejects_sent_envelope_with_session_id_but_root_url() {
    let mut envelope = sent_envelope(proven_turns(true));
    envelope["conversationUrl"] = json!("https://chatgpt.com/");

    let error = confirm_send_started(&envelope).expect_err("root url rejected");
    assert_eq!(error, ConfirmationError::UrlSessionMismatch);
}

#[test]
fn rejects_sent_envelope_with_url_session_mismatch() {
    let mut envelope = sent_envelope(proven_turns(true));
    envelope["conversationUrl"] = json!("https://chatgpt.com/c/other-session");

    let error = confirm_send_started(&envelope).expect_err("mismatch rejected");
    assert_eq!(error, ConfirmationError::UrlSessionMismatch);
}

#[test]
fn rejects_sent_envelope_without_target_mapping() {
    let mut envelope = sent_envelope(proven_turns(true));
    envelope.as_object_mut().expect("object").remove("targetId");

    let error = confirm_send_started(&envelope).expect_err("target required");
    assert_eq!(error, ConfirmationError::MissingEvidence("targetId"));
}

#[test]
fn rejects_sent_envelope_without_both_server_turn_identities() {
    let error = confirm_send_started(&sent_envelope(json!({
        "activeTurn": true,
        "userTurnId": format!("turn_{}", "1".repeat(64)),
        "assistantTurnId": null
    })))
    .expect_err("both stable turn identities required");

    assert_eq!(error, ConfirmationError::NoRealTurnEvidence);
}

#[test]
fn rejects_legacy_boolean_or_count_only_turn_evidence() {
    let error = confirm_send_started(&sent_envelope(json!({
        "activeTurn": true,
        "newUserTurn": true,
        "newAssistantTurn": true,
        "userCount": 1,
        "assistantCount": 1
    })))
    .expect_err("boolean/count evidence is not a turn identity");

    assert_eq!(error, ConfirmationError::NoRealTurnEvidence);
}

#[test]
fn confirms_terminal_answer_only_with_session_url_target_answer_and_assistant_turn() {
    let confirmation = confirm_terminal_answer(&done_envelope("Final answer")).expect("terminal");

    assert_eq!(confirmation.session_id, "sid-confirm");
    assert_eq!(
        confirmation.conversation_url,
        "https://chatgpt.com/c/sid-confirm"
    );
    assert_eq!(confirmation.target_id, "target-abc");
    assert_eq!(confirmation.answer_text_len, "Final answer".len());
}

#[test]
fn rejects_terminal_answer_with_root_url() {
    let mut envelope = done_envelope("Final answer");
    envelope["conversationUrl"] = json!("https://chatgpt.com/");

    let error = confirm_terminal_answer(&envelope).expect_err("root terminal rejected");
    assert_eq!(error, ConfirmationError::UrlSessionMismatch);
}

#[test]
fn rejects_terminal_answer_with_url_session_mismatch() {
    let mut envelope = done_envelope("Final answer");
    envelope["conversationUrl"] = json!("https://chatgpt.com/c/other-session");

    let error = confirm_terminal_answer(&envelope).expect_err("mismatch rejected");
    assert_eq!(error, ConfirmationError::UrlSessionMismatch);
}

#[test]
fn rejects_terminal_answer_with_empty_answer_text() {
    let error = confirm_terminal_answer(&done_envelope("   ")).expect_err("empty answer rejected");
    assert_eq!(error, ConfirmationError::EmptyAnswer);
}

#[test]
fn rejects_terminal_answer_without_assistant_turn_hash() {
    let mut envelope = done_envelope("Final answer");
    envelope["assistantTurn"]["textSha256"] = json!("");

    let error = confirm_terminal_answer(&envelope).expect_err("assistant hash required");
    assert_eq!(
        error,
        ConfirmationError::MissingEvidence("assistantTurn.textSha256")
    );
}
