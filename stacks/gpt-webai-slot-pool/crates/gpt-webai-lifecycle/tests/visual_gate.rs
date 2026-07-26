use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::visual_gate::{confirm_pre_send_visual_gate, VisualGateError};
use serde_json::json;

#[test]
fn accepts_ready_status_and_saved_capture_diagnostics() {
    confirm_pre_send_visual_gate(&ready_status(), &captured()).expect("visual gate");
}

#[test]
fn rejects_status_without_saved_screenshot_and_dom() {
    let mut status = ready_status();
    status["diagnostics"]["screenshot"] = json!("failed");

    let error = confirm_pre_send_visual_gate(&status, &captured()).expect_err("missing screenshot");
    assert_eq!(error, VisualGateError::StatusDiagnosticsMissing);
}

#[test]
fn rejects_login_readiness_signal_before_send() {
    let mut status = ready_status();
    status["diagnostics"]["readinessSignals"]["login"] = json!(true);

    let error = confirm_pre_send_visual_gate(&status, &captured()).expect_err("login blocked");
    assert_eq!(error, VisualGateError::ReadinessSignal("login"));
}

#[test]
fn rejects_non_captured_visual_evidence() {
    let mut capture = captured();
    capture["status"] = json!("capture_failed");

    let error = confirm_pre_send_visual_gate(&ready_status(), &capture).expect_err("capture");
    assert_eq!(error, VisualGateError::CaptureMissing);
}

#[test]
fn accepts_root_composer_pre_send_without_scrollbar_proof_before_session_exists() {
    let mut status = root_ready_status();
    status["diagnostics"]["scrollBottomProof"] = json!({
        "schema": "gpt-webai.scroll-bottom-proof.v1",
        "status": "unverified",
        "reason": "scrollport_not_found",
        "fullViewportScreenshot": { "status": "saved", "path": "/diagnostics/status.png" },
        "rightEdgeScrollbarCrop": {
            "status": "saved",
            "path": "/diagnostics/status.right-edge-scrollbar.png"
        },
        "visualScrollbarProof": {
            "status": "unavailable",
            "alignment": { "status": "unavailable" }
        }
    });
    let mut capture = root_captured();
    capture["diagnostics"]["scrollBottomProof"] =
        status["diagnostics"]["scrollBottomProof"].clone();

    confirm_pre_send_visual_gate(&status, &capture).expect("root composer pre-send gate");
}

#[test]
fn accepts_root_composer_pre_send_without_scroll_bottom_proof_field() {
    let mut status = root_ready_status();
    status["diagnostics"]
        .as_object_mut()
        .unwrap()
        .remove("scrollBottomProof");
    let mut capture = root_captured();
    capture["diagnostics"]
        .as_object_mut()
        .unwrap()
        .remove("scrollBottomProof");

    confirm_pre_send_visual_gate(&status, &capture).expect("root composer without proof field");
}

#[test]
fn accepts_root_composer_pre_send_using_envelope_url_when_diagnostics_url_missing() {
    let mut status = root_ready_status();
    status["diagnostics"].as_object_mut().unwrap().remove("url");
    status["diagnostics"]["scrollBottomProof"]["status"] = json!("unverified");
    status["diagnostics"]["scrollBottomProof"]["reason"] = json!("scrollport_not_found");
    let mut capture = root_captured();
    capture["diagnostics"]
        .as_object_mut()
        .unwrap()
        .remove("url");
    capture["diagnostics"]["scrollBottomProof"] =
        status["diagnostics"]["scrollBottomProof"].clone();

    confirm_pre_send_visual_gate(&status, &capture).expect("root composer envelope URL fallback");
}

#[test]
fn rejects_unverified_scrollbar_proof_once_conversation_url_exists() {
    let mut status = root_ready_status();
    status["diagnostics"]["url"] = json!("https://chatgpt.com/c/sid-existing");
    status["diagnostics"]["scrollBottomProof"]["status"] = json!("unverified");
    status["diagnostics"]["scrollBottomProof"]["reason"] =
        json!("right_edge_scrollbar_thumb_bottom_gap");
    let mut capture = root_captured();
    capture["conversationUrl"] = json!("https://chatgpt.com/c/sid-existing");
    capture["diagnostics"]["url"] = json!("https://chatgpt.com/c/sid-existing");

    let error = confirm_pre_send_visual_gate(&status, &capture).expect_err("post-session proof");
    assert_eq!(error, VisualGateError::BottomScrollUnverified("status"));
}

#[test]
fn rejects_unverified_scrollbar_proof_when_assistant_turn_exists() {
    let mut status = root_ready_status();
    status["diagnostics"]["selectorInventory"]["assistantTurns"] = json!(1);
    status["diagnostics"]["assistantTurns"] = json!([{
        "index": 0,
        "tag": "article",
        "textLength": 42
    }]);
    status["diagnostics"]["scrollBottomProof"]["status"] = json!("unverified");
    let capture = root_captured();

    let error = confirm_pre_send_visual_gate(&status, &capture).expect_err("assistant proof");
    assert_eq!(error, VisualGateError::BottomScrollUnverified("status"));
}

#[test]
fn rejects_scroll_unverified_status_explicitly() {
    let mut status = ready_status();
    status["diagnostics"]["scrollBottomProof"]["status"] = json!("unverified");
    status["diagnostics"]["scrollBottomProof"]["reason"] =
        json!("right_edge_scrollbar_thumb_bottom_gap");

    let error = confirm_pre_send_visual_gate(&status, &captured()).expect_err("scroll proof");
    assert_eq!(error, VisualGateError::BottomScrollUnverified("status"));
}

#[test]
fn rejects_scroll_unverified_capture_status_explicitly() {
    let mut capture = captured();
    capture["status"] = json!("scroll.bottom_unverified");
    capture["reason"] = json!("scroll.bottom_unverified");

    let error = confirm_pre_send_visual_gate(&ready_status(), &capture).expect_err("scroll proof");
    assert_eq!(error, VisualGateError::BottomScrollUnverified("capture"));
}

fn root_ready_status() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "ready",
        "url": "https://chatgpt.com/",
        "diagnostics": root_saved_diagnostics()
    })
}

fn root_captured() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "conversationUrl": "https://chatgpt.com/",
        "diagnostics": root_saved_diagnostics()
    })
}

fn root_saved_diagnostics() -> serde_json::Value {
    json!({
        "screenshot": "saved",
        "dom": "saved",
        "sessionId": "",
        "url": "https://chatgpt.com/",
        "readinessSignals": {
            "login": false,
            "limit": false,
            "pro": true,
            "composer": true,
            "stopControls": 0
        },
        "selectorInventory": {
            "assistantTurns": 0
        },
        "assistantTurns": [],
        "scrollBottomProof": verified_scroll_bottom_proof()
    })
}

fn ready_status() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "ready",
        "diagnostics": saved_diagnostics()
    })
}

fn captured() -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "sessionId": "sid-visual-gate-fixture",
        "conversationUrl": "https://chatgpt.com/c/sid-visual-gate-fixture",
        "diagnostics": saved_diagnostics()
    })
}

fn saved_diagnostics() -> serde_json::Value {
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
            "path": "/diagnostics/status.png"
        },
        "rightEdgeScrollbarCrop": {
            "status": "saved",
            "path": "/diagnostics/status.right-edge-scrollbar.png"
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
