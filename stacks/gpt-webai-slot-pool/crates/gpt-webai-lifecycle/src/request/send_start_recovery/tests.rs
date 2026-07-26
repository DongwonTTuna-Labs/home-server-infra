use super::*;
use crate::provider_runner::ProviderPathMode;

#[test]
fn refuses_to_promote_dom_stop_controls_without_server_turn_identities() {
    let candidate = CandidateEvidence {
        path: PathBuf::from("send-after-start-confirmation.dom.json"),
        value: serde_json::json!({
            "schema": "gpt-webai-provider-dom-diagnostics.v1",
            "label": "send-after-start-confirmation",
            "sessionId": "sid-a",
            "url": "https://chatgpt.com/c/sid-a",
            "readinessSignals": {"stopControls": 1},
            "selectorInventory": {"assistantTurns": 0}
        }),
    };
    let recovered = parse_candidate(candidate).expect("recovered");
    assert!(matches!(recovered, SendStartRecovery::Unconfirmed(_)));
}

#[test]
fn recovers_only_explicit_target_and_both_server_turn_identities() {
    let candidate = CandidateEvidence {
        path: PathBuf::from("send-start-confirmation.json"),
        value: serde_json::json!({
            "schema": "gpt-webai.send-start-confirmation.v1",
            "sessionId": "sid-a",
            "conversationUrl": "https://chatgpt.com/c/sid-a",
            "targetId": "target-a",
            "turnEvidence": {
                "activeTurn": true,
                "userTurnId": format!("turn_{}", "1".repeat(64)),
                "assistantTurnId": format!("turn_{}", "2".repeat(64))
            }
        }),
    };
    let recovered = parse_candidate(candidate).expect("recovered");
    let SendStartRecovery::Confirmed(confirmed) = recovered else {
        panic!("expected confirmed recovery");
    };
    assert_eq!(confirmed.start.target_id, "target-a");
    assert_eq!(
        confirmed.start.user_turn_id,
        format!("turn_{}", "1".repeat(64))
    );
    assert_eq!(
        confirmed.start.assistant_turn_id,
        format!("turn_{}", "2".repeat(64))
    );
}

#[test]
fn root_url_is_not_confirmed() {
    let candidate = CandidateEvidence {
        path: PathBuf::from("send-start-confirmation.json"),
        value: serde_json::json!({
            "sessionId": "sid-a",
            "conversationUrl": "https://chatgpt.com/",
            "turnEvidence": {"activeTurn": true}
        }),
    };
    let recovered = parse_candidate(candidate).expect("recovered");
    assert!(matches!(recovered, SendStartRecovery::Unconfirmed(_)));
}

#[test]
fn artifact_dirs_include_docker_and_host_env() {
    let command = ProviderCommand {
        provider_bin: PathBuf::from("provider"),
        args_prefix: Vec::new(),
        env: vec![(
            "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
            "/tmp/host-artifacts".to_string(),
        )],
        path_mode: ProviderPathMode::DockerSlot(crate::provider_runner::DockerSlotPaths {
            artifact_host_dir: PathBuf::from("/tmp/docker-artifacts"),
            artifact_container_dir: "/broker-artifacts/run".to_string(),
            attachment_host_dir: PathBuf::from("/tmp/attachments"),
            attachment_container_dir: "/broker-attachments/run".to_string(),
        }),
    };
    let dirs = artifact_dirs(&command);
    assert!(dirs.contains(&PathBuf::from("/tmp/docker-artifacts")));
    assert!(dirs.contains(&PathBuf::from("/tmp/host-artifacts")));
}
