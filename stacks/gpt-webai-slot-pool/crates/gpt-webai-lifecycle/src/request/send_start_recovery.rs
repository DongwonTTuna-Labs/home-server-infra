use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::confirmation::SendStartConfirmation;
use crate::provider_runner::{ProviderCommand, ProviderPathMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SendStartRecovery {
    Confirmed(SendStartRecoveryEvidence),
    Unconfirmed(SendStartUnconfirmedEvidence),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SendStartRecoveryEvidence {
    pub start: SendStartConfirmation,
    pub source_path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SendStartUnconfirmedEvidence {
    pub session_id: Option<String>,
    pub conversation_url: Option<String>,
    pub source_path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CandidateEvidence {
    path: PathBuf,
    value: Value,
}

pub(crate) fn recover_send_start_from_artifacts(
    command: &ProviderCommand,
) -> Option<SendStartRecovery> {
    let candidates = send_start_candidates(command);
    let mut unconfirmed = None;
    for candidate in candidates {
        let parsed = parse_candidate(candidate);
        match parsed {
            Some(SendStartRecovery::Confirmed(evidence)) => {
                return Some(SendStartRecovery::Confirmed(evidence));
            }
            Some(SendStartRecovery::Unconfirmed(evidence)) if unconfirmed.is_none() => {
                unconfirmed = Some(evidence);
            }
            _ => {}
        }
    }
    unconfirmed.map(SendStartRecovery::Unconfirmed)
}

fn send_start_candidates(command: &ProviderCommand) -> Vec<CandidateEvidence> {
    let mut paths = Vec::new();
    for artifact_dir in artifact_dirs(command) {
        let diagnostics_dir = artifact_dir.join("diagnostics");
        paths.push(diagnostics_dir.join("send-start-confirmation.json"));
        if let Ok(entries) = fs::read_dir(&diagnostics_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if file_name.contains("send-after-start-confirmation")
                    && file_name.ends_with(".dom.json")
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|path| {
            let text = fs::read_to_string(&path).ok()?;
            let value = serde_json::from_str::<Value>(&text).ok()?;
            Some(CandidateEvidence { path, value })
        })
        .collect()
}

fn artifact_dirs(command: &ProviderCommand) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let ProviderPathMode::DockerSlot(paths) = &command.path_mode {
        dirs.push(paths.artifact_host_dir.clone());
    }
    for (key, value) in &command.env {
        if (key == "GPT_WEBAI_ARTIFACTS_HOST_DIR" || key == "GPT_WEBAI_ARTIFACTS_DIR")
            && !value.trim().is_empty()
        {
            dirs.push(PathBuf::from(value));
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

fn parse_candidate(candidate: CandidateEvidence) -> Option<SendStartRecovery> {
    if !looks_like_send_start(&candidate) {
        return None;
    }
    let (session_id, conversation_url) = session_and_url(&candidate.value)?;
    if !conversation_url_matches_session(&conversation_url, &session_id) {
        return Some(SendStartRecovery::Unconfirmed(
            SendStartUnconfirmedEvidence {
                session_id: Some(session_id),
                conversation_url: Some(conversation_url),
                source_path: candidate.path,
                message: "durable send-start evidence has a non-/c or mismatched conversation URL"
                    .to_string(),
            },
        ));
    }

    let target_id = string_field(&candidate.value, &["targetId"]);
    let turn_evidence = confirmed_turn_evidence(&candidate.value);
    if let (Some(target_id), Some((active_turn, user_turn_id, assistant_turn_id))) =
        (target_id, turn_evidence)
    {
        return Some(SendStartRecovery::Confirmed(SendStartRecoveryEvidence {
            start: SendStartConfirmation {
                session_id,
                conversation_url,
                target_id,
                active_turn,
                user_turn_id,
                assistant_turn_id,
            },
            source_path: candidate.path,
            message: "recovered confirmed send-start evidence from durable provider diagnostics"
                .to_string(),
        }));
    }

    Some(SendStartRecovery::Unconfirmed(
        SendStartUnconfirmedEvidence {
            session_id: Some(session_id),
            conversation_url: Some(conversation_url),
            source_path: candidate.path,
            message:
                "durable send-start evidence lacks targetId or both server-assigned turn identities"
                    .to_string(),
        },
    ))
}

fn looks_like_send_start(candidate: &CandidateEvidence) -> bool {
    let file_name = candidate
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file_name == "send-start-confirmation.json"
        || file_name.contains("send-after-start-confirmation")
    {
        return true;
    }
    let label = string_field(&candidate.value, &["label"]);
    matches!(
        label.as_deref(),
        Some("send-after-start-confirmation" | "send-start-confirmation")
    )
}

fn session_and_url(value: &Value) -> Option<(String, String)> {
    let url = string_field(value, &["conversationUrl", "url"])?;
    let session = string_field(value, &["sessionId"]).or_else(|| session_id_from_url(&url))?;
    Some((session, url))
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    for name in names {
        let candidate = value
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty());
        if let Some(candidate) = candidate {
            return Some(candidate.to_string());
        }
    }
    None
}

fn confirmed_turn_evidence(value: &Value) -> Option<(bool, String, String)> {
    let turn_evidence = value.get("turnEvidence").and_then(Value::as_object);
    let active_turn = turn_evidence
        .and_then(|object| object.get("activeTurn"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let user_turn_id = turn_evidence
        .and_then(|object| object.get("userTurnId"))
        .and_then(Value::as_str)
        .filter(|value| crate::contracts::ids::validate_turn_id(value).is_ok())?;
    let assistant_turn_id = turn_evidence
        .and_then(|object| object.get("assistantTurnId"))
        .and_then(Value::as_str)
        .filter(|value| crate::contracts::ids::validate_turn_id(value).is_ok())?;
    Some((
        active_turn,
        user_turn_id.to_string(),
        assistant_turn_id.to_string(),
    ))
}

fn conversation_url_matches_session(url: &str, session_id: &str) -> bool {
    url == format!("https://chatgpt.com/c/{session_id}")
}

fn session_id_from_url(url: &str) -> Option<String> {
    let prefix = "https://chatgpt.com/c/";
    let suffix = url.strip_prefix(prefix)?;
    if suffix.is_empty() || suffix.contains('/') || suffix.contains('?') || suffix.contains('#') {
        return None;
    }
    Some(suffix.to_string())
}

#[cfg(test)]
mod tests;
