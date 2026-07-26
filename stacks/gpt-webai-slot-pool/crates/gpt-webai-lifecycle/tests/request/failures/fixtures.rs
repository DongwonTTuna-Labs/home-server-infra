use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::runtime::{
    DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe,
};
use serde_json::json;

pub(super) fn sent(session_id: &str) -> serde_json::Value {
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

pub(super) fn exited_runtime() -> StaticRuntimeProbe {
    StaticRuntimeProbe::new([
        (
            "slot-01".to_string(),
            RuntimeObservation {
                docker_status: DockerStatus::Exited,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        ),
        (
            "slot-02".to_string(),
            RuntimeObservation {
                docker_status: DockerStatus::Exited,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        ),
    ])
}
