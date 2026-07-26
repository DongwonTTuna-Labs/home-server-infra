use gpt_webai_lifecycle::provider_client::{
    validate_provider_envelope, ProviderContractError, PROVIDER_SCHEMA,
};
use serde_json::json;

#[test]
fn accepts_terminal_provider_envelope_with_structured_artifact_object() {
    let envelope = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": "sid",
        "conversationUrl": "https://chatgpt.com/c/sid",
        "answerText": "VERDICT: LGTM_NO_BLOCKING",
        "artifacts": [{
            "sessionId": "sid",
            "buttonText": "pr72-review.zip",
            "buttonTextSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "role": "button",
                "tag": "button"
            },
            "artifact": {
                "status": "saved",
                "hostPath": "/tmp/evidence/pr72-review.zip",
                "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "size": 123
            }
        }],
        "artifactCandidates": []
    });

    let summary = validate_provider_envelope(&envelope).expect("valid provider envelope");
    assert_eq!(summary.status, "done");
    assert_eq!(summary.artifacts, 1);
    assert_eq!(summary.artifact_candidates, 0);
}

#[test]
fn rejects_artifact_object_without_visible_button_text_identity() {
    let envelope = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "artifact.download_timeout",
        "sessionId": "sid",
        "conversationUrl": "https://chatgpt.com/c/sid",
        "artifacts": [],
        "artifactCandidates": [{
            "buttonText": "",
            "buttonTextSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "clickedElement": {},
            "artifact": {
                "status": "failed",
                "reason": "artifact.download_timeout"
            }
        }]
    });

    let error = validate_provider_envelope(&envelope).expect_err("invalid artifact identity");
    assert_eq!(
        error,
        ProviderContractError::InvalidArtifactObject {
            array: "artifactCandidates",
            index: 0,
            field: "buttonText",
        }
    );
}

#[test]
fn accepts_sent_envelope_with_root_url_for_later_confirmation_rejection() {
    let envelope = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "sent",
        "sessionId": "sid",
        "conversationUrl": "https://chatgpt.com/"
    });

    let summary = validate_provider_envelope(&envelope).expect("shape-valid provider envelope");
    assert_eq!(summary.status, "sent");
    assert_eq!(summary.session_id.as_deref(), Some("sid"));
    assert_eq!(
        summary.conversation_url.as_deref(),
        Some("https://chatgpt.com/")
    );
}

#[test]
fn accepts_artifact_expectation_taxonomy_and_session_content_unavailable() {
    for expectation in ["none", "optional", "required", "claimed"] {
        let envelope = json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": "done",
            "sessionId": "sid",
            "conversationUrl": "https://chatgpt.com/c/sid",
            "answerText": "final answer",
            "artifactExpectation": expectation
        });
        let summary = validate_provider_envelope(&envelope).expect("valid expectation");
        assert_eq!(summary.artifact_expectation.as_deref(), Some(expectation));
    }

    let unavailable = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "session.content_unavailable",
        "reason": "session.content_unavailable",
        "sessionId": "sid"
    });
    assert_eq!(
        validate_provider_envelope(&unavailable)
            .expect("known content unavailable status")
            .status,
        "session.content_unavailable"
    );
}

#[test]
fn rejects_unknown_artifact_expectation() {
    let envelope = json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": "sid",
        "conversationUrl": "https://chatgpt.com/c/sid",
        "answerText": "final answer",
        "artifactExpectation": "sometimes"
    });

    let error = validate_provider_envelope(&envelope).expect_err("invalid expectation");
    assert_eq!(
        error,
        ProviderContractError::InvalidField("artifactExpectation")
    );
}

#[test]
fn accepts_scroll_bottom_unverified_statuses() {
    for status in ["scroll.bottom_unverified", "session.running_unverified"] {
        let envelope = json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": status,
            "reason": "scroll.bottom_unverified",
            "sessionId": "sid",
            "conversationUrl": "https://chatgpt.com/c/sid"
        });

        let summary = validate_provider_envelope(&envelope).expect("known scroll status");
        assert_eq!(summary.status, status);
        assert_eq!(summary.session_id.as_deref(), Some("sid"));
    }
}

#[test]
fn rejects_scroll_bottom_unverified_statuses_without_session_id() {
    for status in ["scroll.bottom_unverified", "session.running_unverified"] {
        let envelope = json!({
            "schema": PROVIDER_SCHEMA,
            "ok": true,
            "vendor": "chatgpt",
            "status": status,
            "reason": "scroll.bottom_unverified",
            "conversationUrl": "https://chatgpt.com/c/sid"
        });

        let error = validate_provider_envelope(&envelope).expect_err("missing session id");
        assert_eq!(error, ProviderContractError::InvalidField("sessionId"));
    }
}
