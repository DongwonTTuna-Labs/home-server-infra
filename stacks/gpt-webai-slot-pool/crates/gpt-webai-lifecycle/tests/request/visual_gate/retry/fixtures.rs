use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use serde_json::json;

pub(super) fn write_visual_provider(
    root: &Path,
    args_log: &Path,
    first_status: &Path,
    ready_status: &Path,
    capture: &Path,
) -> PathBuf {
    let path = root.join("visual-provider.sh");
    let status_count = root.join("status.count");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\n\
             printf '%s ' \"$@\" >> '{}'\n\
             printf '\\n' >> '{}'\n\
             case \"$1\" in\n\
             status)\n\
               count=0\n\
               [ -f '{}' ] && count=$(cat '{}')\n\
               count=$((count + 1))\n\
               printf '%s\\n' \"$count\" > '{}'\n\
               if [ \"$count\" -eq 1 ]; then cat '{}'; else cat '{}'; fi\n\
               ;;\n\
             capture) cat '{}' ;;\n\
             send) cat \"$FAKE_PROVIDER_SEND_JSON\" ;;\n\
             poll) cat \"$FAKE_PROVIDER_POLL_JSON\" ;;\n\
             *) printf '{{\"schema\":\"{}\",\"ok\":false,\"vendor\":\"chatgpt\",\"status\":\"provider.schema_drift\",\"reason\":\"provider.schema_drift\"}}\\n'; exit 2 ;;\n\
             esac\n",
            args_log.display(),
            args_log.display(),
            status_count.display(),
            status_count.display(),
            status_count.display(),
            first_status.display(),
            ready_status.display(),
            capture.display(),
            PROVIDER_SCHEMA
        ),
    )
    .expect("write visual provider");
    set_executable(&path);
    path
}

pub(super) fn command_count(args: &str, prefix: &str) -> usize {
    args.lines().filter(|line| line.starts_with(prefix)).count()
}

pub(super) fn status(status: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": false,
        "vendor": "chatgpt",
        "status": status
    })
}

pub(super) fn ready_status() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "ready",
        "diagnostics": diagnostics()
    })
}

pub(super) fn captured() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "diagnostics": diagnostics()
    })
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

fn diagnostics() -> serde_json::Value {
    json!({
        "screenshot": "saved",
        "dom": "saved",
        "readinessSignals": {
            "login": false,
            "limit": false,
            "pro": true,
            "composer": true
        },
        "scrollBottomProof": verified_scroll_bottom_proof()
    })
}

fn verified_scroll_bottom_proof() -> serde_json::Value {
    json!({
        "schema": "gpt-webai.scroll-bottom-proof.v1",
        "status": "verified",
        "verificationMode": "strict_visible_right_edge_scrollbar",
        "fullViewportScreenshot": {
            "status": "saved",
            "path": "/diagnostics/capture.png"
        },
        "rightEdgeScrollbarCrop": {
            "status": "saved",
            "path": "/diagnostics/capture.right-edge-scrollbar.png"
        },
        "visualScrollbarProof": {
            "status": "right_edge_scrollbar_at_bottom",
            "alignment": {
                "status": "bottom_aligned",
                "thumbBottomGapPx": 6,
                "allowedBottomGapPx": 14
            }
        },
        "visibleRightEdgeScrollbarProof": {
            "status": "verified",
            "method": "strict_visible_right_edge_scrollbar",
            "observations": {
                "screenshot": "right_edge_scrollbar_at_bottom",
                "dom": "right_edge_scrollbar_at_bottom",
                "pixel": "right_edge_scrollbar_at_bottom"
            }
        },
        "moreContentAffordances": {
            "status": "clear",
            "count": 0,
            "samples": []
        },
        "consistency": {
            "status": "consistent",
            "screenshotSelected": {
                "selectionKind": "chatgpt_scroll_root_scrollbar"
            },
            "domSelected": {
                "selectionKind": "chatgpt_scroll_root_scrollbar"
            }
        }
    })
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}
