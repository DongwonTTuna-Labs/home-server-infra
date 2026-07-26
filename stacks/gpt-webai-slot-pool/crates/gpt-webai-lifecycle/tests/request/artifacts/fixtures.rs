use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use serde_json::json;

pub fn sent(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "sent",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "turnEvidence": {
            "activeTurn": true,
            "userTurnId": format!("turn_{}", "1".repeat(64)),
            "assistantTurnId": format!("turn_{}", "2".repeat(64))
        }
    })
}

pub fn done_with_candidate(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer with artifact",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "artifacts": [],
        "artifactCandidates": [{
            "sessionId": session_id,
            "buttonText": "pr72-artifact.zip",
            "buttonTextSha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "role": "button",
                "tag": "button"
            },
            "artifact": {
                "status": "failed",
                "reason": "artifact.pending_download"
            }
        }]
    })
}

pub fn done_no_artifacts(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer without artifact",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "artifacts": [],
        "artifactCandidates": []
    })
}

pub fn download_done(session_id: &str, saved_artifacts: usize) -> serde_json::Value {
    let artifacts = if saved_artifacts == 0 {
        Vec::new()
    } else {
        vec![json!({
            "sessionId": session_id,
            "buttonText": "pr72-artifact.zip",
            "buttonTextSha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "role": "button",
                "tag": "button"
            },
            "artifact": {
                "status": "saved",
                "hostPath": "/tmp/evidence/pr72-artifact.zip",
                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "size": 123
            }
        })]
    };
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "artifacts": artifacts,
        "artifactCandidates": []
    })
}

pub fn download_recovery_failed(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": false,
        "vendor": "chatgpt",
        "status": "artifact.recovery_failed",
        "reason": "artifact.recovery_failed",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "artifacts": [],
        "artifactCandidates": [{
            "sessionId": session_id,
            "buttonText": "pr72-artifact.zip",
            "buttonTextSha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "turnScope": "current-assistant-turn",
            "clickedElement": {
                "role": "button",
                "tag": "button"
            },
            "artifact": {
                "status": "failed",
                "reason": "artifact.recovery_failed"
            }
        }]
    })
}
