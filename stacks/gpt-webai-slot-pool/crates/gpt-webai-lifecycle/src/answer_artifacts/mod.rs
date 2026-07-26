use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::json;

use crate::config::SupervisorConfig;
use crate::confirmation::TerminalAnswerConfirmation;
use crate::provider_runner::ProviderPathMode;

pub(crate) struct AnswerArtifactContext<'a> {
    pub config: &'a SupervisorConfig,
    pub path_mode: &'a ProviderPathMode,
    pub request_id: &'a str,
    pub run_id: &'a str,
    pub terminal: &'a TerminalAnswerConfirmation,
}

pub(crate) fn write_answer_artifacts(context: AnswerArtifactContext<'_>) -> io::Result<()> {
    let dir = answer_artifact_dir(&context);
    crate::provider_runner::create_private_directory(&context.config.state_root, &dir)?;
    fs::write(dir.join("answer.md"), &context.terminal.answer_text)?;
    let answer = json!({
        "schema": "gpt-webai.answer-artifact.v1",
        "requestId": context.request_id,
        "runId": context.run_id,
        "sessionId": context.terminal.session_id,
        "conversationUrl": context.terminal.conversation_url,
        "targetId": context.terminal.target_id,
        "answerText": context.terminal.answer_text,
        "answerTextLen": context.terminal.answer_text_len,
        "answerTextSha256": context.terminal.answer_text_sha256,
        "assistantTurnTextSha256": context.terminal.assistant_turn_text_sha256,
    });
    let bytes = serde_json::to_vec_pretty(&answer).map_err(io::Error::other)?;
    fs::write(dir.join("answer.json"), bytes)
}

fn answer_artifact_dir(context: &AnswerArtifactContext<'_>) -> PathBuf {
    match context.path_mode {
        ProviderPathMode::DockerSlot(paths) => paths.artifact_host_dir.clone(),
        ProviderPathMode::Host => context
            .config
            .state_root
            .join("requests")
            .join(safe_key(context.run_id))
            .join("artifacts"),
    }
}

fn safe_key(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = safe.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}
