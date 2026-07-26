use gpt_webai_lifecycle::contracts::browser::{
    ControlIdentity, Effort, EffortProof, EvidenceMediaType, EvidenceRef, FailureProof, Model,
    ModelProof,
};
use gpt_webai_lifecycle::model_selection::{
    validate_failure_proof, validate_success_proofs, ModelSelectionError,
};

#[test]
fn both_legal_model_effort_tuples_require_exact_success_proofs() {
    for (model, effort) in [(Model::Pro, Effort::Standard), (Model::Xhigh, Effort::High)] {
        validate_success_proofs(&model_proof(model), &effort_proof(effort)).expect("legal tuple");
    }

    let mut mismatch = model_proof(Model::Pro);
    mismatch.observed = Model::Xhigh;
    assert_eq!(
        validate_success_proofs(&mismatch, &effort_proof(Effort::Standard)),
        Err(ModelSelectionError::Invalid("proof identity"))
    );
    assert_eq!(
        validate_success_proofs(&model_proof(Model::Pro), &effort_proof(Effort::High)),
        Err(ModelSelectionError::Invalid("requested tuple"))
    );
}

#[test]
fn selection_proofs_reject_stale_or_non_visible_controls_and_duplicate_evidence() {
    let mut disabled = model_proof(Model::Pro);
    disabled.control.disabled = true;
    assert_eq!(
        validate_success_proofs(&disabled, &effort_proof(Effort::Standard)),
        Err(ModelSelectionError::Invalid("model control"))
    );

    let mut duplicate = effort_proof(Effort::Standard);
    duplicate
        .evidence_refs
        .push(duplicate.evidence_refs[0].clone());
    assert_eq!(
        validate_success_proofs(&model_proof(Model::Pro), &duplicate),
        Err(ModelSelectionError::Invalid("duplicate evidenceRefs"))
    );
}

#[test]
fn failure_proof_accepts_only_the_closed_reason_set_with_private_evidence_refs() {
    for reason in [
        "picker.model_absent",
        "picker.effort_absent",
        "picker.control_drift",
        "picker.selection_timeout",
        "picker.reverify_mismatch",
        "capture.ambiguous",
    ] {
        validate_failure_proof(&FailureProof {
            reason: reason.to_string(),
            picker_opened: true,
            requested_model_visible: false,
            requested_effort_visible: false,
            control_identity_stable: true,
            evidence_refs: vec![evidence()],
            failed_at_ms: 1,
        })
        .expect("closed failure reason");
    }

    let invalid = FailureProof {
        reason: "picker.pro_absent".to_string(),
        picker_opened: true,
        requested_model_visible: false,
        requested_effort_visible: false,
        control_identity_stable: true,
        evidence_refs: vec![evidence()],
        failed_at_ms: 1,
    };
    assert_eq!(
        validate_failure_proof(&invalid),
        Err(ModelSelectionError::Invalid("reason"))
    );
}

fn model_proof(model: Model) -> ModelProof {
    ModelProof {
        requested: model.clone(),
        observed: model,
        verified: true,
        control: control(),
        selected_by: "already_exact".to_string(),
        evidence_refs: vec![evidence()],
        verified_at_ms: 1,
    }
}

fn effort_proof(effort: Effort) -> EffortProof {
    EffortProof {
        requested: effort.clone(),
        observed: effort,
        verified: true,
        control: control(),
        selected_by: "picker".to_string(),
        evidence_refs: vec![evidence()],
        verified_at_ms: 1,
    }
}

fn control() -> ControlIdentity {
    ControlIdentity {
        bounding_box_hash: h('1'),
        control_id: id("control", '2'),
        disabled: false,
        dom_path_hash: h('3'),
        label_hash: h('4'),
        role: "button".to_string(),
        test_id_hash: None,
        visible: true,
    }
}

fn evidence() -> EvidenceRef {
    EvidenceRef {
        path: "dom.sanitized.json".to_string(),
        sha256: h('a'),
        size_bytes: 1,
        media_type: EvidenceMediaType::Json,
    }
}

fn h(value: char) -> String {
    format!("sha256:{}", value.to_string().repeat(64))
}

fn id(prefix: &str, value: char) -> String {
    format!("{prefix}_{}", value.to_string().repeat(64))
}
