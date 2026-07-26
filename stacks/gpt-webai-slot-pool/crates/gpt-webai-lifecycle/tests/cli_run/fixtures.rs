use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

pub(super) fn stdout_json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).expect("stdout json")
}

pub(super) struct Fixture {
    pub root: PathBuf,
    pub prompt: PathBuf,
    pub upload_one: PathBuf,
    pub upload_two: PathBuf,
    pub docker_log: PathBuf,
    pub provider_operations: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        let root = temp_root(prefix);
        let prompt = root.join("prompt.md");
        fs::write(&prompt, "hello from fake docker cli run").expect("prompt");
        let upload_one = root.join("upload-one.txt");
        fs::write(&upload_one, "first upload").expect("upload one");
        let upload_two = root.join("upload-two.md");
        fs::write(&upload_two, "second upload").expect("upload two");
        let docker_log = root.join("docker.log");
        let provider_operations = root.join("provider-operations.log");
        Self {
            root,
            prompt,
            upload_one,
            upload_two,
            docker_log,
            provider_operations,
        }
    }

    pub(super) fn write_fake_docker(&self) -> PathBuf {
        let provider = self.root.join("r13-fake-provider.py");
        fs::write(&provider, R13_FAKE_PROVIDER).expect("write R13 fake provider");
        let mut provider_permissions = fs::metadata(&provider)
            .expect("provider metadata")
            .permissions();
        provider_permissions.set_mode(0o600);
        fs::set_permissions(&provider, provider_permissions).expect("provider chmod");
        let path = self.root.join("fake-docker.sh");
        fs::write(
            &path,
            format!(
                r#"#!/usr/bin/env bash
set -euo pipefail
root='{root}'
provider='{provider}'
printf '%s\n' "$*" >> '{docker_log}'
case "${{1:-}}" in
  compose)
    case "$*" in
      *" up -d --force-recreate gpt-webai-slot-01")
        printf '%s\n%s\n%s\n' \
          "$PR72_OWNER_ID" "$PR72_OWNER_GENERATION" "$PR72_RUNTIME_INCARNATION" \
          > "$root/runtime-identity"
        printf 'running\n' > "$root/runtime-status"
        ;;
      *" stop gpt-webai-slot-01")
        printf 'exited\n' > "$root/runtime-status"
        if [[ "${{GPT_WEBAI_TEST_BREAK_RELEASE_JOURNAL:-}}" == 1 ]]; then
          chmod 000 "$root/journal/events"
        fi
        ;;
      *) exit 2 ;;
    esac
    ;;
  inspect)
    IFS= read -r owner < "$root/runtime-identity"
    generation="$(sed -n '2p' "$root/runtime-identity")"
    incarnation="$(sed -n '3p' "$root/runtime-identity")"
    status="$(cat "$root/runtime-status")"
    if [[ "$status" == exited ]]; then
      finished='"2026-07-24T00:01:00Z"'
    else
      finished='null'
    fi
    printf '[{{"Config":{{"Labels":{{"pr72.gpt-webai.owner-generation":"%s","pr72.gpt-webai.owner-id":"%s","pr72.gpt-webai.runtime-incarnation":"%s"}}}},"Id":"%064d","Name":"/gpt-webai-slot-01","State":{{"ExitCode":0,"FinishedAt":%s,"StartedAt":"2026-07-24T00:00:00Z","Status":"%s"}}}}]\n' \
      "$generation" "$owner" "$incarnation" 0 "$finished" "$status"
    ;;
  exec)
    request_path=''
    while [[ $# -gt 0 ]]; do
      if [[ "$1" == --request-file && $# -ge 2 ]]; then
        request_path="$2"
        break
      fi
      shift
    done
    [[ "$request_path" == /state/slot-01/* ]] || exit 3
    host_path="$root/slots/slot-01/state/${{request_path#/state/slot-01/}}"
    exec /usr/bin/python3 "$provider" --request-file "$host_path" --state-root "$root"
    ;;
  *) exit 2 ;;
esac
"#,
                root = self.root.display(),
                provider = provider.display(),
                docker_log = self.docker_log.display(),
            ),
        )
        .expect("write fake docker");
        set_executable(&path);
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_root(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-cli-run-{prefix}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod");
}

const R13_FAKE_PROVIDER: &str = r#"#!/usr/bin/python3
import hashlib
import json
import os
import sys

def canonical(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode("utf-8") + b"\n"

def sha(value):
    return hashlib.sha256(value).hexdigest()

def h256(value):
    return "sha256:" + sha(value)

def derived(prefix, value):
    return prefix + "_" + sha(canonical(value))

def write_new(path, data):
    parent = os.path.dirname(path)
    os.makedirs(parent, mode=0o700, exist_ok=True)
    os.chmod(parent, 0o700)
    with open(path, "xb") as stream:
        stream.write(data)
    os.chmod(path, 0o600)

def evidence(path):
    data = ("fixture:" + path).encode("utf-8")
    return {"mediaType":"application/json","path":path,"sha256":h256(data),"sizeBytes":len(data)}

def control(label):
    label_hash = h256((label + ":label").encode("utf-8"))
    dom_hash = h256((label + ":dom").encode("utf-8"))
    box_hash = h256((label + ":box").encode("utf-8"))
    return {
        "boundingBoxHash":box_hash,
        "controlId":derived("control", ["button","button","",label_hash,dom_hash,box_hash]),
        "disabled":False,
        "domPathHash":dom_hash,
        "labelHash":label_hash,
        "role":"button",
        "testIdHash":None,
        "visible":True,
    }

def root_id(label, role):
    label_hash = h256((label + ":label").encode("utf-8"))
    dom_hash = h256((label + ":dom").encode("utf-8"))
    box_hash = h256((label + ":box").encode("utf-8"))
    return derived("root", ["main",role,"",label_hash,dom_hash,box_hash])

def receipt(operation, operation_id, request_id, run_id, session_id, payload):
    value = {
        "createdAtMs":1000,
        "operation":operation,
        "operationId":operation_id,
        "payload":payload,
        "receiptId":"",
        "requestId":request_id,
        "runId":run_id,
        "schema":"pr72.receipt.r13.v1",
        "sessionId":session_id,
    }
    value["receiptId"] = "receipt_" + sha(canonical(value))
    return value

request_path = sys.argv[sys.argv.index("--request-file") + 1]
state_root = sys.argv[sys.argv.index("--state-root") + 1]
with open(request_path, "rb") as stream:
    request = json.load(stream)

operation = request["operation"]
identity = request["identity"]
operation_data = request["operationData"]
with open(os.path.join(state_root, "provider-operations.log"), "a", encoding="utf-8") as stream:
    stream.write(operation + "\n")

browser_guid = "12345678-1234-1234-1234-123456789abc"
raw_target = "fixture-target"
page_id = derived("page", ["pr72.page.r13.v1", browser_guid, raw_target, "main-frame", "loader-fixture"])
browser_context_id = derived("ctx", ["pr72.ctx.r13.v1", browser_guid, "fixture-context"])
target_id = derived("target", ["pr72.target.r13.v1", browser_guid, raw_target])
fixture_evidence = evidence("cdp.sanitized.json")

if operation == "status":
    data = {"composerReady":True,"dockerStatus":"running","healthStatus":"ready","modelLabel":"pro","retryAfterMs":None}
elif operation == "capture.root":
    data = {
        "failureProof":None,
        "rootBindingCandidate":{
            "browserContextId":browser_context_id,
            "capturedAtMs":2000,
            "composerRootId":root_id("composer", "form"),
            "conversationRootId":root_id("conversation", "main"),
            "domMutationGeneration":1,
            "effortControl":control("effort"),
            "evidenceRefs":[fixture_evidence],
            "modelControl":control("model"),
            "normalizedUrl":"https://chatgpt.com/",
            "operationId":identity["operationId"],
            "pageIncarnationId":page_id,
            "selectorMargin":100,
            "targetId":target_id,
        },
    }
elif operation == "ensure-model":
    page = operation_data["pageBinding"]
    data = {
        "effortProof":{
            "control":control("effort"),"evidenceRefs":[fixture_evidence],
            "observed":operation_data["requestedEffort"],"requested":operation_data["requestedEffort"],
            "selectedBy":"already_exact","verified":True,"verifiedAtMs":3000,
        },
        "failureProof":None,
        "modelProof":{
            "control":control("model"),"evidenceRefs":[fixture_evidence],
            "observed":operation_data["requestedModel"],"requested":operation_data["requestedModel"],
            "selectedBy":"already_exact","verified":True,"verifiedAtMs":3000,
        },
        "observedPageBinding":page,
    }
elif operation == "upload-only":
    page = operation_data["pageBinding"]
    chips = []
    for record in operation_data["attachmentSet"]["records"]:
        stem = record["containerRelPath"].rsplit("/", 1)[-1].rsplit(".", 1)[0].casefold()
        stem_hash = sha(stem.encode("utf-8"))
        chips.append({
            "boundingBoxHash":h256((stem + ":box").encode("utf-8")),
            "chipStableKey":h256(canonical(["pr72.chip.r13.v1", page["pageIncarnationId"], stem_hash, 0])),
            "complete":True,
            "digest":record["sourceSha256"],
            "evidenceRefs":[fixture_evidence],
            "labelHash":h256((stem + ":label").encode("utf-8")),
            "visibleSizeBytes":record["sizeBytes"],
        })
    data = {
        "failureReason":None,
        "observedPageBinding":page,
        "uploadProof":{
            "allExpectedComplete":True,
            "capturedAtMs":4000,
            "expectedSetSha256":operation_data["attachmentSet"]["setSha256"],
            "retryIndex":operation_data["retryIndex"],
            "staleChips":[],
            "uploadAttemptId":operation_data["uploadAttemptId"],
            "visibleCurrentChips":chips,
        },
    }
elif operation == "send-click":
    page = operation_data["pageBinding"]
    attempt = operation_data["sendAttemptId"]
    prompt_sha = operation_data["promptInput"]["sha256"]
    session_id = "sid-cli-docker"
    user_turn = derived("turn", ["pr72.turn.r13.v1", session_id, "user", "user-message-fixture"])
    assistant_turn = derived("turn", ["pr72.turn.r13.v1", session_id, "assistant", "assistant-message-fixture"])
    pre = {
        "assistantTurnId":None,"capturedAtMs":5000,"conversationUrl":None,
        "evidenceRefs":[evidence("send.pre-click.json")],"kind":"pre_click",
        "pageBinding":page,"physicalClickCount":0,"promptSha256":prompt_sha,
        "sendAttemptId":attempt,"sessionId":None,"userTurnId":None,
    }
    post = {
        "assistantTurnId":assistant_turn,"capturedAtMs":6000,
        "conversationUrl":"https://chatgpt.com/c/" + session_id,
        "evidenceRefs":[evidence("send.post-click.json")],"kind":"post_click",
        "pageBinding":page,"physicalClickCount":1,"promptSha256":prompt_sha,
        "sendAttemptId":attempt,"sessionId":session_id,"userTurnId":user_turn,
    }
    data = {"observedPageBinding":page,"preClickReceipt":pre,"terminalSendReceipt":post}
    for name, stage_operation, payload, stage_session in [
        ("send.pre-click.receipt.json", "send.pre_click", pre, None),
        ("send.post-click.receipt.json", "send.post_click", post, session_id),
    ]:
        stage = receipt(stage_operation, identity["operationId"], identity["requestId"], identity["runId"], stage_session, payload)
        write_new(os.path.join(os.path.dirname(request_path), name), canonical(stage))
elif operation == "session-rebind":
    expected = operation_data["expectation"]
    generation = expected["lastKnownPageBindingGeneration"] + 1
    answer = b"final answer"
    answer_sha = h256(answer)
    answer_rel = "answers/" + identity["operationId"] + ".rebind.answer.md"
    artifact_root = os.path.join(state_root, "artifacts", "r-" + identity["requestId"])
    write_new(os.path.join(artifact_root, answer_rel), answer)
    root_hash = h256(canonical({"fixture":"rebind"}))
    rebound_page = {
        "bindingGeneration":generation,
        "bindingId":derived("binding", ["pr72.page-binding.r13.v1", page_id, root_hash]),
        "browserContextId":browser_context_id,
        "cohort":expected["cohort"],
        "domMutationGeneration":2,
        "leaseGeneration":expected["leaseGeneration"],
        "leaseId":expected["leaseId"],
        "pageIncarnationId":page_id,
        "rootBindingHash":root_hash,
        "runtimeIncarnationId":expected["runtimeIncarnationId"],
        "runtimeOwnerGeneration":expected["runtimeOwnerGeneration"],
        "runtimeOwnerId":expected["runtimeOwnerId"],
        "slotId":expected["slotId"],
        "targetId":target_id,
    }
    session_id = expected["sessionId"]
    echo = dict(rebound_page)
    echo.update({
        "activeTurn":False,
        "conversationUrl":expected["conversationUrl"],
        "pageBindingGeneration":generation,
        "requestId":expected["requestId"],
        "runId":expected["runId"],
        "sessionBindingId":derived("binding", ["pr72.session-binding.r13.v1", session_id, expected["slotId"], expected["cohort"]]),
        "sessionId":session_id,
        "terminalAnswerSha256":answer_sha,
        "visibleAssistantTurnId":derived("turn", ["pr72.turn.r13.v1", session_id, "assistant", "assistant-message-fixture"]),
        "visibleUserTurnId":derived("turn", ["pr72.turn.r13.v1", session_id, "user", "user-message-fixture"]),
    })
    data = {
        "expectation":expected,
        "failureReason":None,
        "hydrationObservations":[{
            "evidenceRefs":[fixture_evidence],"observedAtMs":7000,"observedEcho":echo,
            "remainingDeadlineMs":89000,"sequenceIndex":0,"state":"answer_visible",
        }],
        "observedEcho":echo,
        "pageBindingGeneration":generation,
        "terminalAnswer":{
            "answerRelPath":answer_rel,"answerSha256":answer_sha,
            "answerSizeBytes":len(answer),"terminalAssistantTurnId":echo["visibleAssistantTurnId"],
        },
    }
elif operation == "poll":
    expected = operation_data["expected"]
    answer = b"final answer"
    answer_sha = h256(answer)
    answer_rel = "answers/" + identity["operationId"] + ".answer.md"
    artifact_root = os.path.join(state_root, "artifacts", "r-" + identity["requestId"])
    write_new(os.path.join(artifact_root, answer_rel), answer)
    observed = dict(expected)
    observed.update({"activeTurn":False,"terminalAnswerSha256":answer_sha})
    data = {
        "answerRelPath":answer_rel,
        "answerSha256":answer_sha,
        "answerSizeBytes":len(answer),
        "bottomProof":None,
        "expected":expected,
        "observedEcho":observed,
        "pollState":"terminal",
        "terminalAssistantTurnId":observed["visibleAssistantTurnId"],
    }
elif operation == "artifact-discover":
    expected = operation_data["expected"]
    bottom = {"atBottom":True,"capturedAtMs":9000,"evidenceRefs":[fixture_evidence],"method":"dom_terminal_anchor"}
    zero = {
        "artifactClaimId":operation_data["artifactClaimId"],"bottomProof":bottom,
        "capturedAtMs":9000,"controlCount":0,"evidenceRefs":[fixture_evidence],
        "terminalAssistantTurnId":operation_data["terminalAssistantTurnId"],
    }
    data = {"bottomProof":bottom,"controls":[],"failureReason":None,"observedEcho":expected,"zeroControlProof":zero}
else:
    raise SystemExit("unexpected operation: " + operation)

primary = receipt(operation, identity["operationId"], identity["requestId"], identity["runId"], identity["sessionId"], data)
primary_bytes = canonical(primary)
write_new(os.path.join(os.path.dirname(request_path), "provider-receipt.json"), primary_bytes)
response = {
    "identity":identity,
    "ok":True,
    "operation":operation,
    "operationData":data,
    "providerReason":None,
    "receipt":{"mediaType":"application/json","path":"provider-receipt.json","sha256":h256(primary_bytes),"sizeBytes":len(primary_bytes)},
    "schema":"gpt-webai.provider.response.r13.v1",
    "status":"done",
}
sys.stdout.buffer.write(canonical(response))
"#;
