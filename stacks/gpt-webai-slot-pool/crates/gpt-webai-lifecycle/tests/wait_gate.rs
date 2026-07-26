use gpt_webai_lifecycle::provider_client::PROVIDER_SCHEMA;
use gpt_webai_lifecycle::wait_gate::{
    confirm_pre_poll_wait_gate, PrePollWaitGateEvidence, WaitGateError,
};
use serde_json::json;

#[test]
fn accepts_captured_session_with_real_turn_evidence() {
    let capture = captured("sid-wait");
    confirm_pre_poll_wait_gate(evidence(&capture, true)).expect("wait gate");
}

#[test]
fn rejects_missing_real_turn_evidence() {
    let capture = captured("sid-wait");
    let error = confirm_pre_poll_wait_gate(evidence(&capture, false)).expect_err("turn");
    assert_eq!(error, WaitGateError::NoRealTurnEvidence);
}

#[test]
fn rejects_capture_without_saved_screenshot_and_dom() {
    let mut capture = captured("sid-wait");
    capture["diagnostics"]["dom"] = json!("failed");

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("capture");
    assert_eq!(error, WaitGateError::CaptureMissing);
}

#[test]
fn rejects_capture_url_session_mismatch() {
    let capture = captured("other-session");
    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("url");
    assert_eq!(error, WaitGateError::UrlSessionMismatch);
}

#[test]
fn rejects_scroll_unverified_capture_status_explicitly() {
    let mut capture = captured("sid-wait");
    capture["status"] = json!("scroll.bottom_unverified");
    capture["reason"] = json!("scroll.bottom_unverified");

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("scroll proof");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn rejects_verified_scroll_bottom_proof_when_more_content_affordance_visible() {
    let mut capture = captured("sid-wait");
    capture["diagnostics"]["scrollBottomProof"]["moreContentAffordances"] = json!({
        "status": "visible",
        "count": 1,
        "samples": [{
            "tag": "button",
            "labelPreview": "Scroll to bottom",
            "rect": { "x": 534, "y": 558, "width": 36, "height": 36 }
        }]
    });

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("affordance");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn rejects_visible_right_edge_scrollbar_proof_when_selected_roots_mismatch() {
    let mut capture = captured("sid-wait");
    capture["diagnostics"]["scrollBottomProof"]["consistency"] = json!({
        "status": "mismatch",
        "reason": "scroll_root_selection_kind_inconsistent",
        "screenshotSelected": {
            "selectionKind": "chatgpt_scroll_root_scrollbar"
        },
        "domSelected": {
            "selectionKind": "browser_viewport_scrollbar"
        }
    });

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("root mismatch");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn rejects_unverified_scroll_bottom_proof() {
    let mut capture = captured("sid-wait");
    capture["diagnostics"]["scrollBottomProof"]["status"] = json!("unverified");

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("scroll proof");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn accepts_short_conversation_no_scrollbar_proof_with_real_turn_evidence() {
    let mut capture = captured("sid-wait");
    capture["diagnostics"]["scrollBottomProof"] = short_no_scrollbar_proof();

    confirm_pre_poll_wait_gate(evidence(&capture, true)).expect("short no-scrollbar proof");
}

#[test]
fn rejects_short_conversation_no_scrollbar_proof_without_bottom_readiness() {
    let mut capture = captured("sid-wait");
    let mut proof = short_no_scrollbar_proof();
    proof["bottomReadinessEvidence"]["status"] = json!("unverified");
    proof["bottomReadinessEvidence"]["reason"] = json!("bottom_readiness_evidence_missing");
    proof["bottomReadinessEvidence"]["authenticatedComposerReadyAtBottom"] = json!(false);
    proof["bottomReadinessEvidence"]["activeGenerationAtBottom"] = json!(false);
    proof["bottomReadinessEvidence"]["newestTurnAtBottom"] = json!(false);
    proof["noScrollableConversationOverflowProof"]["bottomReadinessEvidence"] =
        proof["bottomReadinessEvidence"].clone();
    capture["diagnostics"]["scrollBottomProof"] = proof;

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("readiness");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn rejects_short_conversation_no_scrollbar_proof_when_session_url_mismatches() {
    let mut capture = captured("sid-wait");
    let mut proof = short_no_scrollbar_proof();
    proof["bottomReadinessEvidence"]["sessionUrlMatches"] = json!(false);
    proof["noScrollableConversationOverflowProof"]["bottomReadinessEvidence"] =
        proof["bottomReadinessEvidence"].clone();
    capture["diagnostics"]["scrollBottomProof"] = proof;

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("session url");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

#[test]
fn rejects_short_conversation_no_scrollbar_proof_when_conversation_affordance_visible() {
    let mut capture = captured("sid-wait");
    let mut proof = short_no_scrollbar_proof();
    proof["moreContentAffordances"] = json!({
        "status": "visible",
        "count": 1,
        "samples": [{
            "tag": "button",
            "labelPreview": "Scroll to bottom",
            "rect": { "x": 492, "y": 558, "width": 36, "height": 36 },
            "match": { "centeredFloatingIcon": true }
        }]
    });
    capture["diagnostics"]["scrollBottomProof"] = proof;

    let error = confirm_pre_poll_wait_gate(evidence(&capture, true)).expect_err("affordance");
    assert_eq!(error, WaitGateError::BottomScrollUnverified);
}

fn evidence(capture: &serde_json::Value, real_turn_evidence: bool) -> PrePollWaitGateEvidence<'_> {
    PrePollWaitGateEvidence {
        capture_value: capture,
        session_id: "sid-wait",
        conversation_url: "https://chatgpt.com/c/sid-wait",
        real_turn_evidence,
    }
}

fn captured(session_id: &str) -> serde_json::Value {
    json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "sessionId": session_id,
        "conversationUrl": format!("https://chatgpt.com/c/{session_id}"),
        "diagnostics": {
            "screenshot": "saved",
            "dom": "saved",
            "scrollBottomProof": verified_scroll_bottom_proof()
        }
    })
}

fn short_no_scrollbar_proof() -> serde_json::Value {
    let bottom_readiness = json!({
        "schema": "gpt-webai.bottom-readiness-evidence.v1",
        "status": "verified",
        "urlKind": "conversation",
        "sessionIdPresent": true,
        "sessionUrlMatches": true,
        "authenticatedComposerReadyAtBottom": true,
        "activeGenerationAtBottom": true,
        "newestTurnAtBottom": false,
        "evidenceKinds": ["authenticated_composer_ready_at_bottom", "active_generation_at_bottom"],
        "composer": {
            "visible": true,
            "nearBottom": true,
            "bottomGapPx": 34,
            "rect": { "x": 230, "y": 615, "width": 642, "height": 54 }
        },
        "activeGenerationControl": {
            "visible": true,
            "nearBottom": true,
            "bottomGapPx": 42,
            "rect": { "x": 827, "y": 623, "width": 39, "height": 39 }
        }
    });
    json!({
        "schema": "gpt-webai.scroll-bottom-proof.v1",
        "status": "verified",
        "verificationMode": "strict_short_no_scrollbar",
        "fullViewportScreenshot": {
            "status": "saved",
            "path": "/diagnostics/send-after-start-confirmation.png"
        },
        "rightEdgeScrollbarCrop": {
            "status": "saved",
            "path": "/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png",
            "width": 24,
            "height": 703
        },
        "visualScrollbarProof": {
            "status": "unavailable",
            "reason": "scrollbar_thumb_not_found_in_right_edge_crop",
            "method": "right_edge_crop_pixel_scan",
            "crop": { "width": 24, "height": 703 },
            "segments": [],
            "alignment": { "status": "unavailable" }
        },
        "noScrollableConversationOverflowProof": {
            "status": "verified",
            "method": "dom_short_conversation_no_scrollbar",
            "observations": {
                "screenshot": "no_scrollable_overflow",
                "dom": "no_scrollable_overflow",
                "rightEdgeScrollbar": "no_visible_scrollbar"
            },
            "bottomReadinessEvidence": bottom_readiness.clone()
        },
        "bottomReadinessEvidence": bottom_readiness,
        "moreContentAffordances": {
            "status": "clear",
            "count": 0,
            "samples": []
        },
        "ignoredMoreContentAffordances": {
            "status": "ignored",
            "count": 6,
            "samples": [{
                "tag": "a",
                "textPreview": "PR72 Scroll Bottom Fix",
                "labelPreview": "PR72 Scroll Bottom Fix",
                "rect": { "x": 6, "y": 696, "width": 233, "height": 36 },
                "ignoredReason": "sidebar_or_navigation"
            }]
        }
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
