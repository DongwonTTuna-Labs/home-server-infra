use gpt_webai_lifecycle::contracts::ids::{
    artifact_host_saved_rel_path, derive_artifact_id, derive_browser_context_id,
    derive_chip_stable_key, derive_download_event_id, derive_page_binding_id,
    derive_page_incarnation_id, derive_runtime_incarnation_id, derive_session_binding_id,
    derive_target_id, derive_turn_id, derive_writer_id,
};
use gpt_webai_lifecycle::contracts::provider::{
    ProviderContractError, ProviderRequest, ProviderResponse,
};
use serde_json::{json, Value};

fn hex(value: char) -> String {
    std::iter::repeat_n(value, 64).collect()
}

fn h(value: char) -> String {
    format!("sha256:{}", hex(value))
}

fn id(prefix: &str, value: char) -> String {
    format!("{prefix}_{}", hex(value))
}

fn evidence_ref(path: &str) -> Value {
    json!({"path":path,"sha256":h('a'),"sizeBytes":1,"mediaType":"application/json"})
}

#[test]
fn r23_identifier_preimages_match_the_node_contract() {
    let browser_guid = "123e4567-e89b-12d3-a456-426614174000";
    assert_eq!(
        derive_browser_context_id(browser_guid, "").unwrap(),
        "ctx_7dde0f135152d9225b2c576feab8679630f2652c732e71b490e4712a70b67c6c"
    );
    assert_eq!(
        derive_target_id(browser_guid, "CDP-target-1").unwrap(),
        "target_459c74e6f9ce5cf02b5f060b7aeed137dc8f1583a87435ce73ac1f4df407ffac"
    );
    let page_id =
        derive_page_incarnation_id(browser_guid, "CDP-target-1", "frame-1", "loader-1").unwrap();
    assert_eq!(
        page_id,
        "page_fe0510c2d2a43663400cece612bd16b5fb00be36dcae1a5db43f40a9dbadeef7"
    );
    assert_eq!(
        derive_page_binding_id(&page_id, &h('1')).unwrap(),
        "binding_3b1ac7140e1668977c734d11f2d62160438eaba91412e182ce660a8ecd1f3860"
    );
    assert_eq!(
        derive_session_binding_id("session_1", "slot-01", "cohort-a").unwrap(),
        "binding_23be62dc78ad5e82173bd26d647daf67d3cd45b79fd6a62203ecea0a40cc1c3f"
    );
    assert_eq!(
        derive_turn_id("session_1", "user", "msg-user-1").unwrap(),
        "turn_0de2d51a188b8b02c7ab87edb91ccb3effc621b2a63d118fd5c9758b96b9d537"
    );
    assert_eq!(
        derive_turn_id("session_1", "assistant", "msg-assistant-1").unwrap(),
        "turn_47398e3ec8f8709aa3ca97554ad9797ecd8ed13f23d53692ca009f85fd61a7dd"
    );
    assert!(derive_turn_id("session_1", "assistant", "").is_err());
}

#[test]
fn r23_runtime_and_writer_preimages_are_exact() {
    assert_eq!(
        derive_writer_id(
            "host_0123456789abcdef0123456789abcdef",
            123,
            1_700_000_000_123,
        )
        .unwrap(),
        "writer_c2bfeaa640b7a95f64bdf9ec8a9d9a5cda7ee947157aebe9e66dca027e3ff33d"
    );
    assert_eq!(
        derive_runtime_incarnation_id("slot-01", "0123456789abcdef0123456789abcdef",).unwrap(),
        "runtime_b8a080b408ec62281bd42d91ba0e859d7a00d79fa98c51d86abff9669d87e5c2"
    );
    assert!(derive_writer_id("host_BAD", 123, 1_700_000_000_123).is_err());
    assert!(derive_runtime_incarnation_id("slot-01", "short").is_err());
}

#[test]
fn r23_chip_download_artifact_and_path_preimages_are_closed() {
    let page_id = "page_fe0510c2d2a43663400cece612bd16b5fb00be36dcae1a5db43f40a9dbadeef7";
    let claim_id = id("artifact_claim", '2');
    let control_id = id("control", '3');
    assert_eq!(
        derive_chip_stable_key(page_id, "Report Final", 0).unwrap(),
        "sha256:35d9d13de5bd457d440a5ce475d9d5e698ce732b19147f8890128fb0b6c1df1c"
    );
    let download_id = derive_download_event_id(page_id, "download-guid-1", "report.zip").unwrap();
    assert_eq!(
        download_id,
        "download_1cbe92311abe1e5d7aa21b91da163e11ac401a760d27235c6a66b3b6d1f9fadc"
    );
    let artifact_id = derive_artifact_id(&claim_id, &control_id, &download_id).unwrap();
    assert_eq!(
        artifact_id,
        "artifact_bbcbc069d97d9f45c9d31bf833f8be22beb621a6bcc117ae7cf53c086eb4aaec"
    );
    assert_eq!(
        artifact_host_saved_rel_path("r-request-1", &claim_id, &artifact_id).unwrap(),
        format!("artifacts/r-request-1/{claim_id}/{artifact_id}.download")
    );
}

fn binding() -> Value {
    let page_incarnation_id = id("page", '4');
    let root_binding_hash = h('5');
    json!({
        "bindingId":derive_page_binding_id(&page_incarnation_id,&root_binding_hash).unwrap(),
        "bindingGeneration":1,"browserContextId":id("ctx",'2'),
        "cohort":"cohort-a","domMutationGeneration":0,"leaseGeneration":1,
        "leaseId":id("lease",'3'),"pageIncarnationId":page_incarnation_id,
        "rootBindingHash":root_binding_hash,
        "runtimeIncarnationId":id("runtime",'6'),"runtimeOwnerGeneration":1,
        "runtimeOwnerId":id("owner",'7'),"slotId":"slot-01","targetId":id("target",'8')
    })
}

fn echo(terminal: bool) -> Value {
    let mut value = binding();
    let object = value.as_object_mut().unwrap();
    object.extend(
        json!({
            "activeTurn":!terminal,"conversationUrl":"https://chatgpt.com/c/session_1",
            "pageBindingGeneration":1,"requestId":"request-1","runId":"run-1",
            "sessionBindingId":derive_session_binding_id("session_1","slot-01","cohort-a").unwrap(),
            "sessionId":"session_1",
            "terminalAnswerSha256":terminal.then(||h('a')),
            "visibleAssistantTurnId":Some(id("turn",'b')),
            "visibleUserTurnId":Some(id("turn",'c'))
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    value
}

fn expectation() -> Value {
    json!({
        "cohort":"cohort-a","conversationUrl":"https://chatgpt.com/c/session_1",
        "lastKnownPageBindingGeneration":0,"leaseGeneration":1,"leaseId":id("lease",'3'),
        "requestId":"request-1","runId":"run-1","runtimeIncarnationId":id("runtime",'6'),
        "runtimeOwnerGeneration":1,"runtimeOwnerId":id("owner",'7'),"sessionId":"session_1",
        "sessionOperationClaimId":id("claim",'d'),"slotId":"slot-01"
    })
}

fn control() -> Value {
    json!({
        "boundingBoxHash":h('1'),"controlId":id("control",'2'),"disabled":false,
        "domPathHash":h('3'),"labelHash":h('4'),"role":"button","testIdHash":null,"visible":true
    })
}

fn artifact_control() -> Value {
    json!({
        "boundingBoxHash":h('1'),"controlId":id("control",'2'),
        "currentTurnId":id("turn",'b'),"disabled":false,"domPathHash":h('3'),
        "role":"button","visible":true,"visibleTextHash":h('4')
    })
}

fn chip() -> Value {
    json!({
        "boundingBoxHash":h('1'),"chipStableKey":h('2'),"complete":true,"digest":h('3'),
        "evidenceRefs":[evidence_ref("dom.sanitized.json")],"labelHash":h('4'),"visibleSizeBytes":1
    })
}

fn upload_proof() -> Value {
    json!({
        "allExpectedComplete":true,"capturedAtMs":1,"expectedSetSha256":h('e'),
        "retryIndex":0,"staleChips":[],"uploadAttemptId":"upload-1","visibleCurrentChips":[]
    })
}

fn send_receipt(kind: &str) -> Value {
    let terminal = kind != "pre_click";
    json!({
        "assistantTurnId":terminal.then(||id("turn",'b')),"capturedAtMs":1,
        "conversationUrl":terminal.then_some("https://chatgpt.com/c/session_1"),
        "evidenceRefs":[evidence_ref("dom.sanitized.json")],"kind":kind,"pageBinding":binding(),
        "physicalClickCount":if kind=="post_click"{1}else{0},"promptSha256":h('f'),
        "sendAttemptId":"send-1","sessionId":terminal.then_some("session_1"),
        "userTurnId":terminal.then(||id("turn",'c'))
    })
}

fn identity(session: bool) -> Value {
    json!({
        "cohort":"cohort-a","operationId":"operation-1","requestId":"request-1",
        "runId":"run-1","sessionId":session.then_some("session_1"),"slotId":"slot-01"
    })
}

fn evidence(operation: &str) -> Value {
    json!({
        "cdpRelPath":"cdp.sanitized.json","domRelPath":"dom.sanitized.json",
        "receiptRelPaths":{
            "primary":"provider-receipt.json",
            "preClick":matches!(operation,"send-click"|"send-reconcile").then_some("send.pre-click.receipt.json"),
            "postClick":(operation=="send-click").then_some("send.post-click.receipt.json"),
            "reconcile":(operation=="send-reconcile").then_some("send.reconcile.receipt.json")
        },
        "screenshotRelPath":"screenshot.privacy-crop.png"
    })
}

fn request_data(operation: &str) -> Value {
    let terminal = operation.starts_with("artifact-");
    match operation {
        "status" => json!({"expectedSlotId":"slot-01","probeAttempt":0}),
        "capture.root" => {
            json!({"requestedModel":"pro","requestedEffort":"standard","rediscoveryAttempt":0})
        }
        "ensure-model" => {
            json!({"pageBinding":binding(),"requestedModel":"pro","requestedEffort":"standard","pickerOpenBudget":1,"stabilizationMs":500})
        }
        "upload-only" => {
            json!({"pageBinding":binding(),"attachmentSet":{"count":0,"records":[],"setSha256":h('e')},"uploadAttemptId":"upload-1","retryIndex":0})
        }
        "clear-upload" => {
            json!({"pageBinding":binding(),"uploadAttemptId":"upload-1","clearAttemptId":"clear-1","staleChips":[chip()]})
        }
        "send-click" => {
            json!({"pageBinding":binding(),"sendAttemptId":"send-1","uploadProof":upload_proof(),"promptInput":{"containerRelPath":"run-1/prompt.txt","sha256":h('f'),"sizeBytes":1},"clickBudget":1})
        }
        "send-reconcile" => {
            json!({"pageBinding":binding(),"sendAttemptId":"send-1","preClickReceipt":send_receipt("pre_click")})
        }
        "session-rebind" => {
            json!({"operationKind":"poll","expectation":expectation(),"navigationAttemptLimit":2,"hydrationDeadlineMs":90000})
        }
        "poll" => {
            json!({"expected":echo(terminal),"pollAttemptId":"poll-1","pollTimeoutSeconds":1,"artifactExpectation":"none"})
        }
        "artifact-discover" => {
            json!({"expected":echo(terminal),"artifactClaimId":id("artifact_claim",'5'),"terminalAssistantTurnId":id("turn",'b'),"expectation":"optional"})
        }
        "artifact-click-save" => {
            let claim_id = id("artifact_claim", '5');
            let directory = format!("artifacts/r-request-1/{claim_id}");
            json!({
                "expected":echo(terminal),"artifactClaimId":claim_id,
                "terminalAssistantTurnId":id("turn",'b'),"control":artifact_control(),
                "baseline":{"baselineSha256":h('6'),"capturedAtMs":1,"directory":directory,"entries":[]},
                "controlIndex":0,"hostSaveDirectory":directory
            })
        }
        _ => unreachable!(),
    }
}

fn request_value(operation: &str) -> Value {
    json!({
        "deadlineMs":1,"evidence":evidence(operation),
        "identity":identity(matches!(operation,"session-rebind"|"poll"|"artifact-discover"|"artifact-click-save")),
        "operation":operation,"operationData":request_data(operation),
        "schema":"gpt-webai.provider.request.r13.v1"
    })
}

fn selection_proof(requested: &str) -> Value {
    json!({
        "control":control(),"evidenceRefs":[evidence_ref("dom.sanitized.json")],
        "observed":requested,"requested":requested,"selectedBy":"already_exact",
        "verified":true,"verifiedAtMs":1
    })
}

fn bottom_proof() -> Value {
    json!({"atBottom":true,"capturedAtMs":1,"evidenceRefs":[evidence_ref("dom.sanitized.json")],"method":"scrollbar"})
}

fn response_data(operation: &str) -> Value {
    match operation {
        "status" => {
            json!({"healthStatus":"ready","dockerStatus":"running","retryAfterMs":null,"modelLabel":"pro","composerReady":true})
        }
        "capture.root" => json!({
            "rootBindingCandidate":{
                "browserContextId":id("ctx",'2'),"capturedAtMs":1,"composerRootId":id("root",'1'),
                "conversationRootId":id("root",'2'),"domMutationGeneration":0,"effortControl":control(),
                "evidenceRefs":[evidence_ref("dom.sanitized.json")],"modelControl":control(),
                "normalizedUrl":"https://chatgpt.com/","operationId":"operation-1",
                "pageIncarnationId":id("page",'4'),"selectorMargin":50,"targetId":id("target",'8')
            },"failureProof":null
        }),
        "ensure-model" => {
            json!({"modelProof":selection_proof("pro"),"effortProof":selection_proof("standard"),"failureProof":null,"observedPageBinding":binding()})
        }
        "upload-only" => {
            json!({"uploadProof":upload_proof(),"failureReason":null,"observedPageBinding":binding()})
        }
        "clear-upload" => {
            json!({"clearAttemptId":"clear-1","clearedChips":[{"chipStableKey":h('2'),"digest":h('3'),"cleared":true}],"observedPageBinding":binding()})
        }
        "send-click" => {
            json!({"preClickReceipt":send_receipt("pre_click"),"terminalSendReceipt":send_receipt("post_click"),"observedPageBinding":binding()})
        }
        "send-reconcile" => {
            json!({"preClickReceipt":send_receipt("pre_click"),"terminalSendReceipt":send_receipt("reconciled_turn_start"),"observedPageBinding":binding()})
        }
        "session-rebind" => {
            let observed = echo(false);
            json!({
                "expectation":expectation(),"observedEcho":observed,"pageBindingGeneration":1,
                "hydrationObservations":[{"sequenceIndex":0,"state":"active_generation_visible","remainingDeadlineMs":90000,"observedEcho":echo(false),"evidenceRefs":[evidence_ref("dom.sanitized.json")],"observedAtMs":1}],
                "terminalAnswer":null,"failureReason":null
            })
        }
        "poll" => {
            json!({"expected":echo(false),"observedEcho":echo(false),"pollState":"running","answerSha256":null,"answerSizeBytes":null,"answerRelPath":null,"terminalAssistantTurnId":null,"bottomProof":null})
        }
        "artifact-discover" => {
            let proof = bottom_proof();
            json!({"controls":[],"bottomProof":proof,"zeroControlProof":{"artifactClaimId":id("artifact_claim",'5'),"terminalAssistantTurnId":id("turn",'b'),"bottomProof":bottom_proof(),"controlCount":0,"evidenceRefs":[evidence_ref("dom.sanitized.json")],"capturedAtMs":1},"failureReason":null,"observedEcho":echo(true)})
        }
        "artifact-click-save" => {
            let claim_id = id("artifact_claim", '5');
            let control_id = id("control", '2');
            let download_event_id = id("download", '7');
            let artifact_id =
                derive_artifact_id(&claim_id, &control_id, &download_event_id).unwrap();
            let host_path =
                artifact_host_saved_rel_path("r-request-1", &claim_id, &artifact_id).unwrap();
            json!({
                "downloadReceipt":{
                    "artifactClaimId":claim_id,"artifactId":artifact_id,
                    "browserContextId":id("ctx",'2'),"clickedAtMs":2,"control":artifact_control(),
                    "conversationUrl":"https://chatgpt.com/c/session_1","downloadEventId":download_event_id,
                    "hostSavedRelPath":host_path,"listenerArmedAtMs":1,"mediaType":"application/octet-stream",
                    "pageIncarnationId":id("page",'4'),"receivedAtMs":3,"sessionId":"session_1",
                    "sha256":h('8'),"sizeBytes":1,"slotId":"slot-01","targetId":id("target",'8'),
                    "terminalAssistantTurnId":id("turn",'b')
                },"failureReason":null,"observedEcho":echo(true)
            })
        }
        _ => unreachable!(),
    }
}

fn response_value(operation: &str) -> Value {
    let request = request_value(operation);
    json!({
        "identity":request["identity"].clone(),"ok":true,"operation":operation,
        "operationData":response_data(operation),"providerReason":null,
        "receipt":evidence_ref("provider-receipt.json"),
        "schema":"gpt-webai.provider.response.r13.v1",
        "status":if operation=="poll"{"running"}else{"done"}
    })
}

fn parse_request(operation: &str) -> ProviderRequest {
    serde_json::from_value(request_value(operation)).unwrap()
}

#[test]
fn all_eleven_request_and_success_response_variants_are_closed_and_valid() {
    for operation in [
        "status",
        "capture.root",
        "ensure-model",
        "upload-only",
        "clear-upload",
        "send-click",
        "send-reconcile",
        "session-rebind",
        "poll",
        "artifact-discover",
        "artifact-click-save",
    ] {
        let request = parse_request(operation);
        request
            .validate()
            .unwrap_or_else(|error| panic!("{operation} request: {error}"));
        let response: ProviderResponse = serde_json::from_value(response_value(operation)).unwrap();
        response
            .validate_for(&request)
            .unwrap_or_else(|error| panic!("{operation} response: {error}"));
    }
}

#[test]
fn diagnostic_status_identity_allows_cli_run_id_without_a_request_id() {
    let mut value = request_value("status");
    value["identity"]["requestId"] = Value::Null;
    value["identity"]["runId"] = json!("preflight-run");
    let request: ProviderRequest = serde_json::from_value(value).expect("diagnostic request");
    request.validate().expect("diagnostic status identity");
}

#[test]
fn rejects_cross_slot_page_binding_and_unsupported_model_effort() {
    let mut slot = request_value("ensure-model");
    slot["operationData"]["pageBinding"]["slotId"] = json!("slot-02");
    let request: ProviderRequest = serde_json::from_value(slot).unwrap();
    assert!(request.validate().is_err());

    let mut tuple = request_value("ensure-model");
    tuple["operationData"]["requestedEffort"] = json!("high");
    let request: ProviderRequest = serde_json::from_value(tuple).unwrap();
    assert!(request.validate().is_err());
}

#[test]
fn rejects_response_binding_drift_and_wrong_receipt_path() {
    let request = parse_request("send-click");
    let mut drift = response_value("send-click");
    drift["operationData"]["observedPageBinding"]["targetId"] = json!(id("target", 'f'));
    let response: ProviderResponse = serde_json::from_value(drift).unwrap();
    assert!(response.validate_for(&request).is_err());

    let request = parse_request("status");
    let mut wrong = response_value("status");
    wrong["receipt"]["path"] = json!("other.json");
    let response: ProviderResponse = serde_json::from_value(wrong).unwrap();
    assert!(response.validate_for(&request).is_err());
}

#[test]
fn page_bound_operation_failures_classify_structural_drift_as_binding_mismatch() {
    let mut cases = Vec::new();

    let mut upload = response_value("upload-only");
    upload["ok"] = json!(false);
    upload["status"] = json!("failed");
    upload["providerReason"] = json!("binding.mismatch");
    upload["operationData"] = json!({
        "uploadProof":null,"failureReason":"binding.mismatch",
        "observedPageBinding":binding()
    });
    cases.push(("upload-only", upload));

    let mut clear = response_value("clear-upload");
    clear["ok"] = json!(false);
    clear["status"] = json!("failed");
    clear["providerReason"] = json!("binding.mismatch");
    clear["operationData"] = json!({
        "clearAttemptId":"clear-1","failureReason":"binding.mismatch",
        "attemptedChipKeys":[h('2')],"clearedChips":[],
        "observedPageBinding":binding()
    });
    cases.push(("clear-upload", clear));

    for operation in ["send-click", "send-reconcile"] {
        let mut send = response_value(operation);
        send["ok"] = json!(false);
        send["status"] = json!("failed");
        send["providerReason"] = json!("binding.mismatch");
        send["operationData"] = json!({
            "preClickReceipt":send_receipt("pre_click"),
            "terminalSendReceipt":null,"observedPageBinding":binding()
        });
        cases.push((operation, send));
    }

    for (operation, mut value) in cases {
        value["operationData"]["observedPageBinding"]["targetId"] = json!(id("target", 'f'));
        let request = parse_request(operation);
        let response: ProviderResponse = serde_json::from_value(value).unwrap();
        assert_eq!(
            response.validate_for(&request),
            Err(ProviderContractError::BindingMismatch),
            "{operation} must preserve the typed binding fence"
        );
    }
}

#[test]
fn session_rebind_failure_rejects_terminal_answer_and_mismatch_without_echo() {
    let request = parse_request("session-rebind");
    let base = json!({
        "identity":request_value("session-rebind")["identity"].clone(),"ok":false,
        "operation":"session-rebind","operationData":{
            "expectation":expectation(),"observedEcho":null,"pageBindingGeneration":null,
            "hydrationObservations":[],"failureReason":"session.url_rejected_mismatch"
        },"providerReason":"session.url_rejected_mismatch","receipt":evidence_ref("provider-receipt.json"),
        "schema":"gpt-webai.provider.response.r13.v1","status":"failed"
    });
    let response: ProviderResponse = serde_json::from_value(base.clone()).unwrap();
    assert!(response.validate_for(&request).is_err());

    let mut extra = base;
    extra["operationData"]["terminalAnswer"] = Value::Null;
    assert!(serde_json::from_value::<ProviderResponse>(extra)
        .unwrap()
        .validate_for(&request)
        .is_err());
}

#[test]
fn binding_mismatch_failure_requires_and_preserves_the_mismatched_echo() {
    let request = parse_request("ensure-model");
    let mut value = response_value("ensure-model");
    value["ok"] = json!(false);
    value["status"] = json!("failed");
    value["providerReason"] = json!("binding.mismatch");
    value["operationData"]["modelProof"] = Value::Null;
    value["operationData"]["effortProof"] = Value::Null;
    value["operationData"]["failureProof"] = Value::Null;
    value["operationData"]["observedPageBinding"]["targetId"] = json!(id("target", 'f'));

    let response: ProviderResponse = serde_json::from_value(value.clone()).unwrap();
    response
        .validate_for(&request)
        .expect("a bound mismatch is a valid failure envelope");

    value["operationData"]["observedPageBinding"] = binding();
    let response: ProviderResponse = serde_json::from_value(value).unwrap();
    assert!(response.validate_for(&request).is_err());
}

#[test]
fn non_null_session_failure_echo_must_still_match_the_expectation() {
    let request = parse_request("poll");
    let mut value = response_value("poll");
    value["ok"] = json!(false);
    value["status"] = json!("failed");
    value["providerReason"] = json!("session.content_unavailable");
    value["operationData"]["pollState"] = json!("failed");
    value["operationData"]["observedEcho"]["targetId"] = json!(id("target", 'f'));

    let response: ProviderResponse = serde_json::from_value(value).unwrap();
    assert!(response.validate_for(&request).is_err());
}

#[test]
fn artifact_failure_echo_is_non_null_and_bound_to_the_expected_session() {
    let request = parse_request("artifact-discover");
    let mut value = response_value("artifact-discover");
    value["ok"] = json!(false);
    value["status"] = json!("failed");
    value["providerReason"] = json!("artifact.bottom_unverified");
    value["operationData"] = json!({
        "controls":[],"bottomProof":null,"zeroControlProof":null,
        "failureReason":"artifact.bottom_unverified","observedEcho":echo(true)
    });
    value["operationData"]["observedEcho"]["targetId"] = json!(id("target", 'f'));
    let response: ProviderResponse = serde_json::from_value(value.clone()).unwrap();
    assert!(response.validate_for(&request).is_err());

    value["operationData"]["observedEcho"] = Value::Null;
    let response: ProviderResponse = serde_json::from_value(value).unwrap();
    assert!(response.validate_for(&request).is_err());

    let request = parse_request("artifact-click-save");
    let mut value = response_value("artifact-click-save");
    value["ok"] = json!(false);
    value["status"] = json!("failed");
    value["providerReason"] = json!("artifact.download_timeout");
    value["operationData"] = json!({
        "downloadReceipt":null,"failureReason":"artifact.download_timeout",
        "observedEcho":null
    });
    let response: ProviderResponse = serde_json::from_value(value).unwrap();
    assert!(response.validate_for(&request).is_err());
}

#[test]
fn status_probe_failure_and_page_unreachable_failure_variants_are_accepted() {
    let request = parse_request("status");
    let mut value = response_value("status");
    value["ok"] = json!(false);
    value["status"] = json!("failed");
    value["providerReason"] = json!("probe.timeout");
    value["operationData"] = json!({
        "healthStatus":"unknown","dockerStatus":"unknown","retryAfterMs":null,
        "modelLabel":"unknown","composerReady":false
    });
    let response: ProviderResponse = serde_json::from_value(value).unwrap();
    response
        .validate_for(&request)
        .expect("a failed probe keeps the closed observation shape");

    let mut cases = Vec::new();

    let mut upload = response_value("upload-only");
    upload["ok"] = json!(false);
    upload["status"] = json!("failed");
    upload["providerReason"] = json!("upload.incomplete");
    upload["operationData"] = json!({
        "uploadProof":null,"failureReason":"upload.incomplete","observedPageBinding":null
    });
    cases.push(("upload-only", upload));

    let mut clear = response_value("clear-upload");
    clear["ok"] = json!(false);
    clear["status"] = json!("failed");
    clear["providerReason"] = json!("upload.chip_removal_failed");
    clear["operationData"] = json!({
        "clearAttemptId":"clear-1","failureReason":"upload.chip_removal_failed",
        "attemptedChipKeys":[h('2')],"clearedChips":[],"observedPageBinding":null
    });
    cases.push(("clear-upload", clear));

    for operation in ["send-click", "send-reconcile"] {
        let mut send = response_value(operation);
        send["ok"] = json!(false);
        send["status"] = json!("failed");
        send["providerReason"] = json!("send.click_timeout");
        send["operationData"] = json!({
            "preClickReceipt":send_receipt("pre_click"),"terminalSendReceipt":null,
            "observedPageBinding":null
        });
        cases.push((operation, send));
    }

    for (operation, value) in cases {
        let request = parse_request(operation);
        let response: ProviderResponse = serde_json::from_value(value).unwrap();
        response
            .validate_for(&request)
            .unwrap_or_else(|error| panic!("{operation} page-unreachable failure: {error}"));
    }
}
