use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use serde_json::json;

pub(super) fn write_sequence_provider(
    root: &Path,
    args_log: &Path,
    send_success: &Path,
) -> PathBuf {
    let path = root.join("retry-provider.sh");
    let count_file = root.join("send.count");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\n\
             printf '%s ' \"$@\" >> '{}'\n\
             printf '\\n' >> '{}'\n\
             case \"$1\" in\n\
             send)\n\
               count=0\n\
               [ -f '{}' ] && count=$(cat '{}')\n\
               count=$((count + 1))\n\
               printf '%s\\n' \"$count\" > '{}'\n\
               if [ \"$count\" -eq 1 ]; then printf '{{not-json\\n'; else cat '{}'; fi\n\
               ;;\n\
             poll) cat \"$FAKE_PROVIDER_POLL_JSON\" ;;\n\
             *) printf '{{\"schema\":\"{}\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}}\\n'; exit 2 ;;\n\
             esac\n",
            args_log.display(),
            args_log.display(),
            count_file.display(),
            count_file.display(),
            count_file.display(),
            send_success.display(),
            PROVIDER_SCHEMA
        ),
    )
    .expect("write sequence provider");
    set_executable(&path);
    path
}

pub(super) fn write_always_bad_provider(root: &Path, args_log: &Path) -> PathBuf {
    let path = root.join("always-bad-provider.sh");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\n\
             printf '%s ' \"$@\" >> '{}'\n\
             printf '\\n' >> '{}'\n\
             case \"$1\" in\n\
             send) printf '{{not-json\\n' ;;\n\
             *) printf '{{\"schema\":\"{}\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}}\\n'; exit 2 ;;\n\
             esac\n",
            args_log.display(),
            args_log.display(),
            PROVIDER_SCHEMA
        ),
    )
    .expect("write bad provider");
    set_executable(&path);
    path
}

pub(super) fn write_durable_recovery_provider(
    root: &Path,
    args_log: &Path,
    session_id: &str,
    with_turn_evidence: bool,
) -> PathBuf {
    let path = root.join(format!("durable-recovery-provider-{session_id}.sh"));
    let durable_source = root.join(format!("durable-send-start-{session_id}.json"));
    fs::write(
        &durable_source,
        json!({
            "schema": "gpt-webai.send-start-confirmation.v1",
            "sessionId": session_id,
            "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
            "targetId": "target-run",
            "turnEvidence": {
                "activeTurn": with_turn_evidence,
                "userTurnId": with_turn_evidence.then(|| format!("turn_{}", "1".repeat(64))),
                "assistantTurnId": with_turn_evidence.then(|| format!("turn_{}", "2".repeat(64)))
            },
            "readinessSignals": {
                "stopControls": if with_turn_evidence { 1 } else { 0 }
            },
            "selectorInventory": {
                "assistantTurns": 0
            }
        })
        .to_string(),
    )
    .expect("write durable source");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\n\
             printf '%s ' \"$@\" >> '{}'\n\
             printf '\\n' >> '{}'\n\
             case \"$1\" in\n\
             send)\n\
               mkdir -p \"$GPT_WEBAI_ARTIFACTS_HOST_DIR/diagnostics\"\n\
               cp '{}' \"$GPT_WEBAI_ARTIFACTS_HOST_DIR/diagnostics/send-start-confirmation.json\"\n\
               printf '{{not-json\\n'\n\
               ;;\n\
             poll) cat \"$FAKE_PROVIDER_POLL_JSON\" ;;\n\
             *) printf '{{\"schema\":\"{}\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}}\\n'; exit 2 ;;\n\
             esac\n",
            args_log.display(),
            args_log.display(),
            durable_source.display(),
            PROVIDER_SCHEMA
        ),
    )
    .expect("write durable recovery provider");
    set_executable(&path);
    path
}

pub(super) fn command_count(args: &str, prefix: &str) -> usize {
    args.lines().filter(|line| line.starts_with(prefix)).count()
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}

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

pub(super) fn done(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "done",
        "sessionId": session_id,
        "targetId": "target-run",
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "answerText": "final answer",
        "assistantTurn": {
            "textSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    })
}
