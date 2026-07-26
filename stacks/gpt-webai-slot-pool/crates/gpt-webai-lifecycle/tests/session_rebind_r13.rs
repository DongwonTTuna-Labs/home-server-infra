use gpt_webai_lifecycle::contracts::browser::{
    EvidenceMediaType, EvidenceRef, PageBindingEcho, SessionEcho, SessionRebindExpectation,
};
use gpt_webai_lifecycle::contracts::ids::{
    derive_page_binding_id, derive_session_binding_id, h256,
};
use gpt_webai_lifecycle::session_rebind::hydration::{
    HydrationObservation, HydrationOutcome, HydrationState, HydrationTrace,
};
use gpt_webai_lifecycle::session_rebind::{
    validate_observed_echo, RebindProof, SessionRebindError, TerminalAnswerObservation,
    HYDRATION_DEADLINE_MS, NAVIGATION_ATTEMPT_LIMIT,
};

#[test]
fn validates_pinned_running_rebind_with_fresh_page_generation() {
    assert_eq!(NAVIGATION_ATTEMPT_LIMIT, 2);
    assert_eq!(HYDRATION_DEADLINE_MS, 90_000);
    let expected = expectation();
    let observed = echo(8, true, false);
    let proof = RebindProof {
        expectation: expected.clone(),
        observed_echo: observed.clone(),
        page_binding_generation: 8,
        hydration: trace(observed, HydrationState::ActiveGenerationVisible),
        terminal_answer: None,
    };
    assert_eq!(
        proof.validate(&expected).unwrap(),
        HydrationOutcome::Running
    );
}

#[test]
fn validates_terminal_answer_binding() {
    let expected = expectation();
    let observed = echo(8, false, true);
    let proof = RebindProof {
        expectation: expected.clone(),
        observed_echo: observed.clone(),
        page_binding_generation: 8,
        hydration: trace(observed, HydrationState::AnswerVisible),
        terminal_answer: Some(TerminalAnswerObservation {
            answer_rel_path: "answers/r-request-1/poll-1.answer.md".to_string(),
            answer_sha256: h256(b"answer"),
            answer_size_bytes: 6,
            terminal_assistant_turn_id: turn('b'),
        }),
    };
    assert_eq!(
        proof.validate(&expected).unwrap(),
        HydrationOutcome::Terminal
    );
}

#[test]
fn rejects_root_mismatch_and_wrong_slot_echoes() {
    let expected = expectation();
    let mut observed = echo(8, true, false);
    observed.conversation_url = "https://chatgpt.com/".to_string();
    assert!(validate_observed_echo(&expected, &observed).is_err());

    let mut observed = echo(8, true, false);
    observed.page_binding.slot_id = "slot-02".to_string();
    observed.session_binding_id =
        derive_session_binding_id(session_id(), "slot-02", "cohort-a").unwrap();
    assert!(matches!(
        validate_observed_echo(&expected, &observed),
        Err(SessionRebindError::Invalid("observedEcho identity"))
    ));
}

#[test]
fn rejects_noncontiguous_or_overlong_hydration() {
    let observed = echo(8, true, false);
    let mut invalid_trace = trace(observed.clone(), HydrationState::ActiveGenerationVisible);
    invalid_trace.observations[0].sequence_index = 1;
    assert!(invalid_trace.validate(&observed).is_err());

    let observation = trace(observed.clone(), HydrationState::BlankTransient)
        .observations
        .remove(0);
    let too_many = HydrationTrace {
        observations: vec![observation; 51],
    };
    assert!(too_many.validate(&observed).is_err());
}

#[test]
fn blank_or_content_unavailable_never_establishes_success() {
    let observed = echo(8, false, false);
    assert!(matches!(
        trace(observed.clone(), HydrationState::BlankTransient).validate(&observed),
        Err(SessionRebindError::Invalid("session.hydration_timeout"))
    ));
    assert!(matches!(
        trace(observed.clone(), HydrationState::ContentUnavailable).validate(&observed),
        Err(SessionRebindError::Invalid("session.content_unavailable"))
    ));
}

#[test]
fn persisted_bootstrap_allows_generation_zero_then_observes_one() {
    let mut expected = expectation();
    expected.last_known_page_binding_generation = 0;
    expected.request_id = None;
    expected.run_id = None;
    let observed = echo(1, true, false);
    let proof = RebindProof {
        expectation: expected.clone(),
        observed_echo: observed.clone(),
        page_binding_generation: 1,
        hydration: trace(observed, HydrationState::ActiveGenerationVisible),
        terminal_answer: None,
    };
    assert_eq!(
        proof.validate(&expected).unwrap(),
        HydrationOutcome::Running
    );
}

fn trace(echo: SessionEcho, state: HydrationState) -> HydrationTrace {
    HydrationTrace {
        observations: vec![HydrationObservation {
            sequence_index: 0,
            state,
            remaining_deadline_ms: 89_000,
            observed_echo: echo,
            evidence_refs: vec![EvidenceRef {
                path: "requests/r/operations/rebind/dom.sanitized.json".to_string(),
                sha256: h256(b"dom"),
                size_bytes: 1,
                media_type: EvidenceMediaType::Json,
            }],
            observed_at_ms: 1_000,
        }],
    }
}

fn expectation() -> SessionRebindExpectation {
    SessionRebindExpectation {
        session_id: session_id().to_string(),
        conversation_url: conversation_url(),
        slot_id: "slot-01".to_string(),
        cohort: "cohort-a".to_string(),
        session_operation_claim_id: Some(format!("claim_{}", "1".repeat(64))),
        lease_id: format!("lease_{}", "2".repeat(64)),
        lease_generation: 3,
        runtime_owner_id: format!("owner_{}", "3".repeat(64)),
        runtime_owner_generation: 4,
        runtime_incarnation_id: format!("runtime_{}", "4".repeat(64)),
        request_id: Some("request-1".to_string()),
        run_id: Some("run-1".to_string()),
        last_known_page_binding_generation: 7,
    }
}

fn echo(generation: u16, active: bool, terminal: bool) -> SessionEcho {
    let page_incarnation_id = format!("page_{}", "8".repeat(64));
    let root_binding_hash = h256(b"root");
    SessionEcho {
        page_binding: PageBindingEcho {
            binding_id: derive_page_binding_id(&page_incarnation_id, &root_binding_hash).unwrap(),
            binding_generation: 1,
            slot_id: "slot-01".to_string(),
            cohort: "cohort-a".to_string(),
            lease_id: format!("lease_{}", "2".repeat(64)),
            lease_generation: 3,
            runtime_owner_id: format!("owner_{}", "3".repeat(64)),
            runtime_owner_generation: 4,
            runtime_incarnation_id: format!("runtime_{}", "4".repeat(64)),
            browser_context_id: format!("ctx_{}", "6".repeat(64)),
            target_id: format!("target_{}", "7".repeat(64)),
            page_incarnation_id,
            root_binding_hash,
            dom_mutation_generation: 9,
        },
        session_id: session_id().to_string(),
        conversation_url: conversation_url(),
        request_id: Some("request-1".to_string()),
        run_id: Some("run-1".to_string()),
        session_binding_id: derive_session_binding_id(session_id(), "slot-01", "cohort-a").unwrap(),
        page_binding_generation: generation,
        visible_user_turn_id: (active || terminal).then(|| turn('a')),
        visible_assistant_turn_id: (active || terminal).then(|| turn('b')),
        active_turn: active,
        terminal_answer_sha256: terminal.then(|| h256(b"answer")),
    }
}

fn session_id() -> &'static str {
    "6a623c19-bb00-83ee-bb64-691d8bff937b"
}

fn conversation_url() -> String {
    format!("https://chatgpt.com/c/{}", session_id())
}

fn turn(value: char) -> String {
    format!("turn_{}", value.to_string().repeat(64))
}
