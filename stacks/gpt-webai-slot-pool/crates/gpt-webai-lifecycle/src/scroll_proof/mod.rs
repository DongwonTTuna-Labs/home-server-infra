use serde_json::Value;

pub fn diagnostics_saved(value: &Value) -> bool {
    let Some(diagnostics) = value.get("diagnostics").and_then(Value::as_object) else {
        return false;
    };
    diagnostics.get("screenshot").and_then(Value::as_str) == Some("saved")
        && diagnostics.get("dom").and_then(Value::as_str) == Some("saved")
}

pub fn scroll_bottom_verified(value: &Value) -> bool {
    let Some(diagnostics) = value.get("diagnostics").and_then(Value::as_object) else {
        return false;
    };
    let Some(proof) = diagnostics
        .get("scrollBottomProof")
        .and_then(Value::as_object)
    else {
        return false;
    };
    if proof.get("status").and_then(Value::as_str) != Some("verified") {
        return false;
    }
    if proof
        .get("fullViewportScreenshot")
        .and_then(|screenshot| screenshot.get("status"))
        .and_then(Value::as_str)
        != Some("saved")
    {
        return false;
    }
    if proof
        .get("rightEdgeScrollbarCrop")
        .and_then(|crop| crop.get("status"))
        .and_then(Value::as_str)
        != Some("saved")
    {
        return false;
    }
    if more_content_affordance_visible(proof) {
        return false;
    }
    visible_right_edge_scrollbar_verified(proof)
        || no_scrollable_conversation_overflow_verified(proof)
}

fn visible_right_edge_scrollbar_verified(proof: &serde_json::Map<String, Value>) -> bool {
    if proof.get("verificationMode").and_then(Value::as_str)
        != Some("strict_visible_right_edge_scrollbar")
    {
        return false;
    }
    let Some(visible) = proof
        .get("visibleRightEdgeScrollbarProof")
        .and_then(Value::as_object)
    else {
        return false;
    };
    if visible.get("status").and_then(Value::as_str) != Some("verified") {
        return false;
    }
    if visible.get("method").and_then(Value::as_str) != Some("strict_visible_right_edge_scrollbar")
    {
        return false;
    }
    let Some(observations) = visible.get("observations").and_then(Value::as_object) else {
        return false;
    };
    if observations.get("screenshot").and_then(Value::as_str)
        != Some("right_edge_scrollbar_at_bottom")
    {
        return false;
    }
    if observations.get("dom").and_then(Value::as_str) != Some("right_edge_scrollbar_at_bottom") {
        return false;
    }
    if observations.get("pixel").and_then(Value::as_str) != Some("right_edge_scrollbar_at_bottom") {
        return false;
    }
    let Some(visual) = proof.get("visualScrollbarProof").and_then(Value::as_object) else {
        return false;
    };
    if visual.get("status").and_then(Value::as_str) != Some("right_edge_scrollbar_at_bottom") {
        return false;
    }
    if visual
        .get("alignment")
        .and_then(|alignment| alignment.get("status"))
        .and_then(Value::as_str)
        != Some("bottom_aligned")
    {
        return false;
    }
    let Some(consistency) = proof.get("consistency").and_then(Value::as_object) else {
        return false;
    };
    if consistency.get("status").and_then(Value::as_str) != Some("consistent") {
        return false;
    }
    selected_uses_right_edge_scrollbar(consistency.get("screenshotSelected"))
        && selected_uses_right_edge_scrollbar(consistency.get("domSelected"))
}

fn more_content_affordance_visible(proof: &serde_json::Map<String, Value>) -> bool {
    let Some(affordances) = proof
        .get("moreContentAffordances")
        .and_then(Value::as_object)
    else {
        return false;
    };
    affordances.get("status").and_then(Value::as_str) == Some("visible")
        || affordances
            .get("count")
            .and_then(Value::as_u64)
            .map(|count| count > 0)
            .unwrap_or(false)
        || affordances
            .get("samples")
            .and_then(Value::as_array)
            .map(|samples| !samples.is_empty())
            .unwrap_or(false)
}

fn no_scrollable_conversation_overflow_verified(proof: &serde_json::Map<String, Value>) -> bool {
    if proof.get("verificationMode").and_then(Value::as_str) != Some("strict_short_no_scrollbar") {
        return false;
    }
    let Some(no_overflow) = proof
        .get("noScrollableConversationOverflowProof")
        .and_then(Value::as_object)
    else {
        return false;
    };
    if no_overflow.get("status").and_then(Value::as_str) != Some("verified") {
        return false;
    }
    if no_overflow.get("method").and_then(Value::as_str)
        != Some("dom_short_conversation_no_scrollbar")
    {
        return false;
    }
    let Some(observations) = no_overflow.get("observations").and_then(Value::as_object) else {
        return false;
    };
    if observations.get("screenshot").and_then(Value::as_str) != Some("no_scrollable_overflow") {
        return false;
    }
    if observations.get("dom").and_then(Value::as_str) != Some("no_scrollable_overflow") {
        return false;
    }
    if observations
        .get("rightEdgeScrollbar")
        .and_then(Value::as_str)
        != Some("no_visible_scrollbar")
    {
        return false;
    }
    let Some(visual) = proof.get("visualScrollbarProof").and_then(Value::as_object) else {
        return false;
    };
    if visual.get("status").and_then(Value::as_str) != Some("unavailable") {
        return false;
    }
    if visual.get("reason").and_then(Value::as_str)
        != Some("scrollbar_thumb_not_found_in_right_edge_crop")
    {
        return false;
    }
    bottom_readiness_evidence_verified(
        no_overflow
            .get("bottomReadinessEvidence")
            .or_else(|| proof.get("bottomReadinessEvidence")),
    )
}

fn selected_uses_right_edge_scrollbar(value: Option<&Value>) -> bool {
    let Some(selected) = value.and_then(Value::as_object) else {
        return false;
    };
    let selection_kind = selected
        .get("selectionKind")
        .and_then(Value::as_str)
        .or_else(|| {
            selected
                .get("visualScrollbarProof")
                .and_then(|visual| visual.get("selectionKind"))
                .and_then(Value::as_str)
        });
    matches!(selection_kind, Some("browser_viewport_scrollbar"))
        || matches!(selection_kind, Some("chatgpt_scroll_root_scrollbar"))
}

fn bottom_readiness_evidence_verified(value: Option<&Value>) -> bool {
    let Some(evidence) = value.and_then(Value::as_object) else {
        return false;
    };
    if evidence.get("status").and_then(Value::as_str) != Some("verified") {
        return false;
    }
    if evidence.get("sessionUrlMatches").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    bool_field(evidence, "authenticatedComposerReadyAtBottom")
        || bool_field(evidence, "activeGenerationAtBottom")
        || bool_field(evidence, "newestTurnAtBottom")
}

fn bool_field(map: &serde_json::Map<String, Value>, field: &str) -> bool {
    map.get(field).and_then(Value::as_bool).unwrap_or(false)
}

pub fn scroll_bottom_unverified_reason(value: &Value) -> Option<String> {
    value
        .get("diagnostics")
        .and_then(|diagnostics| diagnostics.get("scrollBottomProof"))
        .and_then(|proof| proof.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string)
}
