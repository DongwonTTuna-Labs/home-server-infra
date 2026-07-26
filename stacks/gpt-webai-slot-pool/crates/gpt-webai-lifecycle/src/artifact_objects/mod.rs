use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::SupervisorConfig;
use crate::provider_runner::ProviderPathMode;

pub struct ArtifactObjectContext<'a> {
    pub config: &'a SupervisorConfig,
    pub path_mode: &'a ProviderPathMode,
    pub request_id: &'a str,
    pub run_id: &'a str,
    pub session_id: &'a str,
    pub conversation_url: &'a str,
    pub slot_id: &'a str,
    pub account_group: &'a str,
}

pub fn write_provider_artifact_objects(
    context: ArtifactObjectContext<'_>,
    provider_value: &Value,
) -> io::Result<()> {
    write_provider_artifact_objects_as(
        context,
        provider_value,
        "provider-download.json",
        "artifact-objects.json",
    )
}

pub fn write_provider_poll_artifact_objects(
    context: ArtifactObjectContext<'_>,
    provider_value: &Value,
) -> io::Result<()> {
    write_provider_artifact_objects_as(
        context,
        provider_value,
        "provider-poll.json",
        "poll-artifact-objects.json",
    )
}

fn write_provider_artifact_objects_as(
    context: ArtifactObjectContext<'_>,
    provider_value: &Value,
    raw_filename: &str,
    manifest_filename: &str,
) -> io::Result<()> {
    let dir = artifact_dir(&context);
    crate::provider_runner::create_private_directory(&context.config.state_root, &dir)?;
    fs::write(
        dir.join(raw_filename),
        serde_json::to_vec_pretty(provider_value).map_err(io::Error::other)?,
    )?;
    let manifest = json!({
        "schema": "gpt-webai.artifact-objects.v1",
        "sessionId": context.session_id,
        "conversationUrl": context.conversation_url,
        "requestId": context.request_id,
        "runId": context.run_id,
        "slotId": context.slot_id,
        "accountGroup": context.account_group,
        "artifacts": provider_value.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "artifactCandidates": provider_value.get("artifactCandidates").cloned().unwrap_or_else(|| json!([])),
        "warnings": provider_value.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "downloadCandidateCount": provider_value.get("downloadCandidateCount").cloned().unwrap_or_else(|| json!(0)),
    });
    fs::write(
        dir.join(manifest_filename),
        serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
    )
}

fn artifact_dir(context: &ArtifactObjectContext<'_>) -> PathBuf {
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
