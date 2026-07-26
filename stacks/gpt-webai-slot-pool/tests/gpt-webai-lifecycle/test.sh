#!/usr/bin/env bash
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stack_root="$(cd -- "$script_dir/../.." && pwd -P)"
repo_root="$(cd -- "$stack_root/../.." && pwd -P)"

usage() {
  printf 'usage: %s <static|fake|full|smoke|all>\n' "$0" >&2
}

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

usage_error() {
  usage
  exit 2
}

require_file() {
  local path="$1"
  [[ -f "$stack_root/$path" && ! -L "$stack_root/$path" ]] ||
    fail "required file is missing or not a regular non-symlink: $path"
}

require_executable() {
  local path="$1"
  require_file "$path"
  [[ -x "$stack_root/$path" ]] || fail "required file is not executable: $path"
}

require_nonempty_directory() {
  local path="$1"
  [[ -d "$stack_root/$path" && ! -L "$stack_root/$path" ]] ||
    fail "required directory is missing or is a symlink: $path"
  find "$stack_root/$path" -type f -print -quit | grep -q . ||
    fail "required directory is empty: $path"
}

assert_focused_acceptance_paths() {
  local target
  local rust_targets=(
    contracts_r13 journal_r13 projection_r13 claims_r13 allocator_r13
    runtime_ownership_r13 model_selection_r13 upload_recovery_r13
    send_reconcile_r13 session_rebind_r13 session_ops_r13 artifact_claims_r13
    release_r13 cli_contract_r13 provider_normalization_r12 qa_counters_r13
  )
  local node_targets=(
    contracts-r13 root-selector-r13 model-selection-r13 upload-only-r13
    send-reconcile-r13 session-rebind-r13 poll-r13 artifact-download-r13
    privacy-evidence-r13 provider-normalization-r12
  )
  for target in "${rust_targets[@]}"; do
    require_file "crates/gpt-webai-lifecycle/tests/${target}.rs"
  done
  for target in "${node_targets[@]}"; do
    require_file "provider/chatgpt-playwright/test/${target}.test.mjs"
  done
}

assert_static_paths() {
  local path
  assert_focused_acceptance_paths
  for path in \
    Cargo.toml \
    crates/gpt-webai-lifecycle/Cargo.toml \
    provider/chatgpt-playwright/package.json \
    provider/chatgpt-playwright/package-lock.json \
    contracts/provider-r12/provider-outcome-current.tsv \
    contracts/provider-r12/provider-outcome-normalized.tsv \
    contracts/provider-r12/r12-to-r13-crosswalk.tsv \
    contracts/ui-labels-r14/model-effort-labels.tsv \
    contracts/ui-labels-r14/chip-removal-labels.tsv \
    scripts/check-provider-normalization-r12.mjs \
    scripts/generate-r12-to-r13-crosswalk.mjs \
    scripts/qa-live-matrix-cases.r13.tsv \
    scripts/qa-live-matrix-r13.sh \
    tests/fixtures/provider-r12/legal-catalog.jsonl \
    tests/fixtures/provider-r12/negative-catalog.jsonl \
    tests/fixtures/provider-r12/semantic-replay.jsonl; do
    require_file "$path"
  done
}

assert_fake_paths() {
  local path
  assert_static_paths
  for path in \
    target/debug/gpt-webai-lifecycle \
    scripts/check-cli-fixtures-r13.py \
    tests/fixtures/cli-r13/accepted.jsonl \
    tests/fixtures/cli-r13/rejected.jsonl; do
    require_file "$path"
  done
  require_executable tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider
  require_nonempty_directory tests/fixtures/lifecycle-r13
}

assert_full_paths() {
  assert_fake_paths
  require_file compose.yaml
  require_file compose.fake.yaml
  require_file Dockerfile
  require_executable scripts/slot-entrypoint.sh
  require_executable scripts/slot-healthcheck.sh
}

run_bash_syntax_checks() {
  local file
  while IFS= read -r file; do
    bash -n "$file"
  done < <(
    find "$stack_root/bin" "$stack_root/scripts" "$stack_root/tests/gpt-webai-lifecycle" \
      -type f \( -name '*.sh' -o -name 'gpt-webai-lifecycle' -o -name 'gpt-webai-lifecycle-rust' \) \
      | LC_ALL=C sort
  )
}

run_source_contract_checks() {
  if grep -RIn --exclude-dir=node_modules --exclude-dir=target 'playwright-core@latest' \
      "$stack_root/Dockerfile" "$stack_root/provider" >/dev/null; then
    fail 'production source must not install playwright-core@latest'
  fi
  if grep -RInE --exclude-dir=target --exclude-dir=node_modules \
      --exclude=qa-file-disposition.r13.tsv --exclude=qa-pr72-final-r13.py \
      '(^|[^[:alnum:]_-])agbrowse([^[:alnum:]_-]|$)' \
      "$stack_root/bin" "$stack_root/scripts" \
      "$stack_root/crates/gpt-webai-lifecycle/src" "$stack_root/provider" \
      "$stack_root/Dockerfile" "$stack_root/compose.yaml" >/dev/null; then
    fail 'production source must not invoke raw agbrowse'
  fi
  if git -C "$repo_root" ls-files -- \
      ':(glob)stacks/gpt-webai-slot-pool/**/node_modules/**' \
      ':(glob)stacks/gpt-webai-slot-pool/**/target/**' \
      ':(glob)stacks/gpt-webai-slot-pool/**/.omo/evidence/**' \
      ':(glob)stacks/gpt-webai-slot-pool/**/.config/chromium/**' \
      ':(glob)stacks/gpt-webai-slot-pool/**/profile/**' | grep -q .; then
    fail 'tracked source includes excluded transient/runtime state'
  fi
}

run_r12_fixture_identity_checks() {
  [[ "$(wc -c < tests/fixtures/provider-r12/legal-catalog.jsonl)" -eq 1478069 ]]
  [[ "$(wc -c < tests/fixtures/provider-r12/negative-catalog.jsonl)" -eq 715667 ]]
  [[ "$(wc -c < tests/fixtures/provider-r12/semantic-replay.jsonl)" -eq 134094 ]]
  [[ "$(wc -c < contracts/provider-r12/r12-to-r13-crosswalk.tsv)" -eq 41367 ]]
  printf '%s  %s\n' \
    fd00c608fe8816fa5fc2086d82c646a1e06b15f96f0b0abf664940309e251ee2 tests/fixtures/provider-r12/legal-catalog.jsonl \
    b21095b36d765f76030f37fe72f87f29d0526550b2c08f685b1a7423e312fee0 tests/fixtures/provider-r12/negative-catalog.jsonl \
    18845a5ff2181a19e5ea0ad23b4fbb7ec3845b84afa20ea69e11930d85722e03 tests/fixtures/provider-r12/semantic-replay.jsonl \
    3c28a1816d5e21fd3bbb6a9afae5e2f9510596dc85a0161ba048b0f3343226fd contracts/provider-r12/r12-to-r13-crosswalk.tsv \
    5fb47aaaf04834d7730088449401ee6c06020576173fb7bf1d45b836673af2d0 contracts/ui-labels-r14/model-effort-labels.tsv \
    5f72d20331679072012c7bfecf7e71dccd6df346c68a4fed3e3e9180782c4b03 contracts/ui-labels-r14/chip-removal-labels.tsv \
    | sha256sum -c -
}

run_r12_crosswalk_regeneration_check() {
  local generated
  generated="$(mktemp)"
  node scripts/generate-r12-to-r13-crosswalk.mjs --output "$generated"
  if ! cmp -s contracts/provider-r12/r12-to-r13-crosswalk.tsv "$generated"; then
    diff -u contracts/provider-r12/r12-to-r13-crosswalk.tsv "$generated" || true
    rm -f -- "$generated"
    fail 'generated R12-to-R13 crosswalk differs from the committed bytes'
  fi
  rm -f -- "$generated"
}

run_live_matrix_catalog_checks() {
  bash -n scripts/qa-live-matrix-r13.sh
  python3 - "$stack_root" <<'PY'
import json
import pathlib
import re
import sys

stack_root = pathlib.Path(sys.argv[1])
catalog_path = stack_root / "scripts/qa-live-matrix-cases.r13.tsv"
fixture_root = stack_root / "tests/fixtures/lifecycle-r13"
contract_path = stack_root / "crates/gpt-webai-lifecycle/src/contracts/cli.rs"


def fail(message):
    raise SystemExit(f"FAIL live-matrix catalog: {message}")


raw = catalog_path.read_bytes()
if raw.startswith(b"\xef\xbb\xbf") or b"\r" in raw or not raw.endswith(b"\n"):
    fail("catalog must be UTF-8 without BOM, LF-only, and final-LF terminated")
try:
    text = raw.decode("utf-8", "strict")
except UnicodeDecodeError as error:
    fail(f"catalog is not UTF-8: {error}")
lines = text.splitlines()
if any(line.endswith((" ", "\t")) for line in lines):
    fail("catalog has trailing whitespace")
header = [
    "caseId",
    "command",
    "argvTemplate",
    "promptId",
    "files",
    "failpoint",
    "expectedResultKinds",
    "repeat10",
    "liveOnly",
]
if not lines or lines[0].split("\t") != header:
    fail("catalog header is not the exact nine-column schema")
rows = [line.split("\t") for line in lines[1:]]
if len(rows) != 31 or any(len(row) != len(header) for row in rows):
    fail("catalog must contain exactly 31 nine-column rows")
case_ids = [row[0] for row in rows]
expected_ids = [f"L{index:02d}" for index in range(1, 22)] + [
    f"R{index:02d}" for index in range(1, 11)
]
if case_ids != sorted(case_ids) or case_ids != expected_ids:
    fail("caseId rows must be unique LC_ALL=C L01-L21 then R01-R10")

source = contract_path.read_text(encoding="utf-8")
matches = re.findall(r'const [A-Z_]+_RESULTS: &str = "(.*?)";', source, re.S)
registry = set()
for value in matches:
    registry.update(value.replace("\\\n", " ").split())
if len(registry) != 98:
    fail(f"closed result registry size is {len(registry)}, expected 98")

allowed_commands = {
    "status",
    "preflight",
    "run",
    "show",
    "resume",
    "download",
    "release",
    "cleanup",
    "state-rebuild",
    "allocate",
}
failpoints = {
    "after-immutable-temp-write",
    "after-immutable-promote-before-directory-fsync",
    "after-event-append-before-head",
    "after-head-before-projection-publish",
    "after-uploadcleared",
    "after-sendclickarmed",
    "after-physical-send-click-before-provider-stdout",
    "after-turnstartconfirmed",
    "after-session-claim-lease-owner-renewal",
    "after-answerterminal",
    "after-artifact-listener-arm",
    "after-artifact-click",
    "after-playwright-host-save-before-receipt",
    "after-receipt-before-event",
    "after-terminalpersisted",
    "after-evidence-preservation",
    "after-runtime-stop-before-resource-release",
    "after-each-exactly-once-release-event",
}

fake_coverage = set()
for path in sorted(fixture_root.glob("*/case.json")):
    try:
        fixture = json.loads(path.read_bytes())
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid lifecycle fixture {path}: {error}")
    kind = fixture.get("expected", {}).get("resultKind")
    if not isinstance(kind, str) or kind not in registry:
        fail(f"fixture has invalid expected.resultKind: {path}")
    fake_coverage.add(kind)

live_union = set()
for row in rows:
    record = dict(zip(header, row, strict=True))
    if record["command"] not in allowed_commands:
        fail(f"{record['caseId']} has an unknown command")
    if record["failpoint"] != "-" and record["failpoint"] not in failpoints:
        fail(f"{record['caseId']} has an unknown failpoint")
    expected = record["expectedResultKinds"].split(",")
    if any(kind not in registry for kind in expected) or len(expected) != len(set(expected)):
        fail(f"{record['caseId']} has invalid or duplicate expected result kinds")
    derived = [kind for kind in expected if kind not in fake_coverage]
    serialized = "-" if not derived else ",".join(derived)
    if record["liveOnly"] != serialized:
        fail(
            f"{record['caseId']} liveOnly={record['liveOnly']} derived={serialized}"
        )
    live_union.update(derived)
    repeat = "true" if record["caseId"].startswith("R") else "false"
    if record["repeat10"] != repeat:
        fail(f"{record['caseId']} repeat10 must be {repeat}")

missing = sorted(registry - fake_coverage - live_union)
if missing:
    fail("result kinds lack fake or live coverage: " + ",".join(missing))
print(
    f"PASS live-matrix catalog rows={len(rows)} registry={len(registry)} "
    f"fakeKinds={len(fake_coverage)} liveKinds={len(live_union)}"
)
PY
}

run_authority_approval_checks() {
  python3 - "$stack_root" <<'PY'
import hashlib
import importlib.util
import pathlib
import tempfile
import sys

stack_root = pathlib.Path(sys.argv[1]).resolve()
source = stack_root / "scripts/qa-pr72-final-r13.py"
spec = importlib.util.spec_from_file_location("qa_pr72_final_r13_test", source)
if spec is None or spec.loader is None:
    raise SystemExit("FAIL authority approval: cannot load final QA verifier")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

with tempfile.TemporaryDirectory(prefix="pr72-authority-approval-") as temporary:
    root = pathlib.Path(temporary)
    authority = root / "authority.txt"
    authority.write_bytes(b"approved authority bytes\n")
    baseline = hashlib.sha256(b"step zero authority bytes\n").hexdigest()
    current = hashlib.sha256(authority.read_bytes()).hexdigest()
    identities = {str(authority): baseline}
    approval = {
        "approvedBy": "fixture",
        "approvedSha256": f"sha256:{current}",
        "baselineSha256": f"sha256:{baseline}",
        "changedAtMs": 1,
        "path": str(authority),
        "reason": "positive authority approval control",
    }
    resolved = []
    observed = module.verify_baseline_identity_map(
        root,
        identities,
        label="authority",
        approved_changes=[approval],
        resolved_changes=resolved,
    )
    if observed != {str(authority): current} or resolved != [approval]:
        raise SystemExit("FAIL authority approval: exact approval did not resolve")

    tampered = dict(approval)
    tampered["approvedSha256"] = "sha256:" + "0" * 64
    try:
        module.verify_baseline_identity_map(
            root,
            identities,
            label="authority",
            approved_changes=[tampered],
        )
    except module.FinalQaError:
        pass
    else:
        raise SystemExit("FAIL authority approval: tampered approvedSha256 was accepted")

print("PASS authority approval exact-match and tampered-hash rejection")
PY
}

run_static_gate() {
  assert_static_paths
  (
    cd "$stack_root"
    export CARGO_NET_OFFLINE=true
    export npm_config_audit=false
    export npm_config_offline=true
    run_source_contract_checks
    cargo fmt --all -- --check
    cargo check --workspace --all-targets --all-features
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo build --workspace
    cargo test --workspace --all-targets --all-features -- --test-threads=1
    npm --prefix provider/chatgpt-playwright ci
    npm --prefix provider/chatgpt-playwright test
    run_r12_fixture_identity_checks
    run_r12_crosswalk_regeneration_check
    run_live_matrix_catalog_checks
    run_authority_approval_checks
    node scripts/check-provider-normalization-r12.mjs \
      --inventory contracts/provider-r12/provider-outcome-current.tsv \
      --catalog contracts/provider-r12/provider-outcome-normalized.tsv \
      --legal-catalog tests/fixtures/provider-r12/legal-catalog.jsonl \
      --negative-catalog tests/fixtures/provider-r12/negative-catalog.jsonl \
      --semantic-replay tests/fixtures/provider-r12/semantic-replay.jsonl
    run_bash_syntax_checks
  )
  git -C "$repo_root" diff --check -- stacks/gpt-webai-slot-pool
}

run_lifecycle_fixture_replay() {
  python3 - "$stack_root" <<'PY'
import base64
import binascii
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
import tempfile

stack_root = pathlib.Path(sys.argv[1]).resolve()
fixture_root = stack_root / "tests/fixtures/lifecycle-r13"
binary = stack_root / "target/debug/gpt-webai-lifecycle"
fake_provider_rel = pathlib.Path(
    "tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider"
)
fake_provider = (stack_root / fake_provider_rel).resolve()
required_cases = {
    "allocate-dry-run",
    "allocate-lock-contended",
    "cleanup-lock-contended",
    "download-completed",
    "download-ambiguous-controls",
    "download-claim-conflict",
    "download-content-unavailable",
    "download-controls-absent-required",
    "download-lock-contended",
    "download-optional-zero",
    "download-event-timeout",
    "download-integrity-failed",
    "download-pinned-slot-unavailable",
    "download-provider-blocked",
    "download-url-rejected",
    "preflight-lock-contended",
    "preflight-state-invalid",
    "release-lock-contended",
    "release-stop-skipped-owner-alive",
    "release-explicit-fenced",
    "release-already-released",
    "release-fencing-mismatch",
    "release-tokenless-takeover",
    "resume-lock-contended",
    "resume-artifact-required-failed",
    "resume-claim-conflict",
    "resume-content-unavailable",
    "resume-pinned-slot-unavailable",
    "resume-provider-blocked",
    "resume-poll-failed",
    "resume-running",
    "resume-terminal-success",
    "resume-terminal-optional-zero",
    "resume-url-rejected",
    "run-lock-contended",
    "run-artifact-required-failed",
    "run-model-failed-drift",
    "run-pool-busy",
    "run-poll-failed",
    "run-running",
    "run-send-failed",
    "run-send-uncertain-reconcile",
    "run-send-uncertain",
    "run-slot-readiness-failed",
    "run-terminal-optional-zero",
    "run-terminal-success",
    "run-upload-stale-clear-retry",
    "run-upload-failed",
    "show-idle",
    "show-content-unavailable",
    "show-claim-conflict",
    "show-lock-contended",
    "show-running",
    "show-provider-blocked",
    "show-pinned-slot-unavailable",
    "show-terminal",
    "show-url-rejected",
    "state-rebuild-check-only",
    "state-rebuild-lock-contended",
    "state-rebuild-match",
    "status-degraded-unknown",
    "status-lock-contended",
}
top_fields = {
    "caseId",
    "env",
    "expected",
    "files",
    "providerScript",
    "schemaVersion",
}
sequence_fields = {"envSequence", "exitSequence"}
expected_fields = {
    "eventTypes",
    "exit",
    "mutates",
    "ok",
    "reason",
    "receiptOps",
    "resultKind",
    "terminal",
}
entry_fields = {"expectOperation", "frame", "malformedBytesB64"}
file_fields = {"contentB64", "relPath"}
identifier = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def fail(message):
    raise SystemExit(f"FAIL lifecycle-r13: {message}")


def canonical_bytes(value):
    return (
        json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        + "\n"
    ).encode("utf-8")


def read_canonical_json(path):
    if path.is_symlink() or not path.is_file():
        fail(f"not a regular non-symlink: {path}")
    raw = path.read_bytes()
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON {path}: {error}")
    if raw != canonical_bytes(value):
        fail(f"non-canonical JSON: {path}")
    return value


def safe_relative(value, label):
    if not isinstance(value, str) or not value or "\\" in value:
        fail(f"invalid {label}")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail(f"unsafe {label}: {value}")
    return path


def materialize_files(work, records):
    if not isinstance(records, list) or len(records) > 8:
        fail("files must contain 0..8 entries")
    seen = set()
    for record in records:
        if not isinstance(record, dict) or set(record) != file_fields:
            fail("files entry has an invalid closed field set")
        relative = safe_relative(record["relPath"], "files[].relPath")
        if str(relative) in seen:
            fail(f"duplicate staged file: {relative}")
        seen.add(str(relative))
        try:
            content = base64.b64decode(record["contentB64"], validate=True)
        except (TypeError, binascii.Error):
            fail(f"invalid contentB64 for {relative}")
        target = work.joinpath(*relative.parts)
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(target.parent, 0o700)
        descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(content)


def resolve_provider_script(case_dir, records, resolved_path):
    if not isinstance(records, list) or len(records) > 32:
        fail("providerScript must contain 0..32 entries")
    resolved = []
    for index, record in enumerate(records):
        if not isinstance(record, dict) or set(record) != entry_fields:
            fail(f"providerScript[{index}] has an invalid closed field set")
        operation = record["expectOperation"]
        if not isinstance(operation, str) or not operation:
            fail(f"providerScript[{index}].expectOperation is invalid")
        frame = record["frame"]
        malformed = record["malformedBytesB64"]
        if (frame is None) == (malformed is None):
            if frame is not None or malformed is not None:
                fail(f"providerScript[{index}] selects two outputs")
        if malformed is not None:
            if not isinstance(malformed, str):
                fail(f"providerScript[{index}].malformedBytesB64 is invalid")
            try:
                base64.b64decode(malformed, validate=True)
            except binascii.Error:
                fail(f"providerScript[{index}].malformedBytesB64 is invalid")
            resolved_frame = None
        elif frame is None:
            resolved_frame = None
        else:
            relative = safe_relative(frame, "providerScript[].frame")
            if relative.parts[0] != "provider-frames":
                fail(f"provider frame is outside provider-frames: {frame}")
            frame_path = case_dir.joinpath(*relative.parts)
            read_canonical_json(frame_path)
            resolved_frame = str(frame_path.resolve())
        resolved.append(
            {
                "expectOperation": operation,
                "frame": resolved_frame,
                "malformedBytesB64": malformed,
            }
        )
    resolved_path.write_bytes(canonical_bytes(resolved))
    os.chmod(resolved_path, 0o600)
    return len(resolved)


def validate_argv(value, label):
    if (
        not isinstance(value, list)
        or not 1 <= len(value) <= 32
        or any(not isinstance(item, str) or "\0" in item for item in value)
    ):
        fail(f"{label} must contain 1..32 strings")
    return list(value)


def validate_env_map(value, label, *, allow_failpoint):
    if not isinstance(value, dict) or list(value) != sorted(value):
        fail(f"{label} must be a sorted object")
    protected = {"GPT_WEBAI_FAKE_SCRIPT", "GPT_WEBAI_STATE_ROOT", "PATH"}
    for name, item in value.items():
        if (
            not identifier.fullmatch(name)
            or not isinstance(item, str)
            or "\0" in item
            or name in protected
            or name == "GPT_WEBAI_FAILPOINT" and not allow_failpoint
        ):
            fail(f"invalid or protected {label} entry {name}")
    return dict(value)


def read_head_event_id(state_root):
    path = state_root / "journal/HEAD.json"
    if not path.exists():
        return None
    head = read_canonical_json(path)
    event_id = head.get("lastEventId")
    if event_id is not None and not isinstance(event_id, str):
        fail("HEAD lastEventId is invalid")
    return event_id


def ordered_head_segment(state_root, before, after):
    if before == after:
        return []
    events = {}
    for path in (state_root / "journal/events").glob("*.json"):
        event = read_canonical_json(path)
        event_id = event.get("eventId")
        if not isinstance(event_id, str) or event_id in events:
            fail(f"duplicate or invalid journal event: {path}")
        events[event_id] = event
    if after not in events or before is not None and before not in events:
        fail("HEAD segment endpoint is absent from the journal")

    indegree = {}
    children = {event_id: [] for event_id in events}
    for event_id, event in events.items():
        dependencies = set(event.get("sourceEventIds", []))
        predecessor = event.get("predecessorEventId")
        if predecessor is not None:
            dependencies.add(predecessor)
        if any(dependency not in events for dependency in dependencies):
            fail(f"journal event has a missing dependency: {event_id}")
        indegree[event_id] = len(dependencies)
        for dependency in dependencies:
            children[dependency].append(event_id)

    ready = sorted(
        (events[event_id]["createdAtMs"], event_id)
        for event_id, degree in indegree.items()
        if degree == 0
    )
    ordered = []
    while ready:
        _, event_id = ready.pop(0)
        ordered.append(event_id)
        for child in children[event_id]:
            indegree[child] -= 1
            if indegree[child] == 0:
                ready.append((events[child]["createdAtMs"], child))
                ready.sort()
    if len(ordered) != len(events):
        fail("journal event dependency cycle")
    start = 0 if before is None else ordered.index(before) + 1
    end = ordered.index(after) + 1
    if end < start:
        fail("HEAD moved backwards across a failpoint step")
    return ordered[start:end]


def command_argv(argv):
    output = list(argv)
    for index, value in enumerate(output[:-1]):
        if value == "--provider-bin" and output[index + 1] == str(fake_provider_rel):
            output[index + 1] = str(fake_provider)
    return output


def receipt_index(state_root):
    index = {}
    for path in state_root.rglob("*.json"):
        if path.is_symlink() or not path.is_file():
            fail(f"unsafe JSON evidence path: {path}")
        try:
            value = json.loads(path.read_bytes())
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        if not isinstance(value, dict) or not isinstance(value.get("receiptId"), str):
            continue
        receipt_id = value["receiptId"]
        operation = value.get("operation")
        if not isinstance(operation, str) or not operation:
            fail(f"receipt without operation: {path}")
        if receipt_id in index:
            fail(f"duplicate receiptId {receipt_id}")
        index[receipt_id] = operation
    return index


def run_case(case_path):
    case_dir = case_path.parent
    case = read_canonical_json(case_path)
    if not isinstance(case, dict):
        fail(f"case root is not an object: {case_path}")
    argv_keys = {name for name in ("argv", "argvSequence") if name in case}
    present_sequence_fields = sequence_fields & set(case)
    if (
        len(argv_keys) != 1
        or "argv" in argv_keys and present_sequence_fields
        or set(case) != top_fields | argv_keys | present_sequence_fields
    ):
        fail(f"invalid closed case field set: {case_path}")
    case_id = case["caseId"]
    if (
        not isinstance(case_id, str)
        or not identifier.fullmatch(case_id)
        or case_id != case_dir.name
    ):
        fail(f"caseId does not match its directory: {case_path}")
    if case["schemaVersion"] != "pr72.lifecycle-fixture.r13.v1":
        fail(f"wrong schemaVersion: {case_id}")
    case_env = validate_env_map(case["env"], f"env for {case_id}", allow_failpoint=False)
    expected = case["expected"]
    if not isinstance(expected, dict) or set(expected) != expected_fields:
        fail(f"expected has an invalid closed field set: {case_id}")
    if (
        not isinstance(expected["exit"], int)
        or isinstance(expected["exit"], bool)
        or not 0 <= expected["exit"] <= 255
        or not isinstance(expected["resultKind"], str)
        or not isinstance(expected["ok"], bool)
        or not isinstance(expected["terminal"], bool)
        or not isinstance(expected["mutates"], bool)
        or expected["reason"] is not None
        and not isinstance(expected["reason"], str)
        or not isinstance(expected["eventTypes"], list)
        or any(not isinstance(item, str) or not item for item in expected["eventTypes"])
        or not isinstance(expected["receiptOps"], list)
        or any(not isinstance(item, str) or not item for item in expected["receiptOps"])
    ):
        fail(f"expected has invalid field values: {case_id}")
    if "argv" in case:
        sequence = [validate_argv(case["argv"], "argv")]
        env_sequence = [{}]
        exit_sequence = [expected["exit"]]
    else:
        if (
            not isinstance(case["argvSequence"], list)
            or not 1 <= len(case["argvSequence"]) <= 16
        ):
            fail(f"argvSequence must contain 1..16 steps: {case_id}")
        sequence = [
            validate_argv(argv, f"argvSequence[{index}]")
            for index, argv in enumerate(case["argvSequence"])
        ]
        raw_env_sequence = case.get("envSequence", [{} for _ in sequence])
        if not isinstance(raw_env_sequence, list) or len(raw_env_sequence) != len(sequence):
            fail(f"envSequence length mismatch: {case_id}")
        env_sequence = [
            validate_env_map(value, f"envSequence[{index}] for {case_id}", allow_failpoint=True)
            for index, value in enumerate(raw_env_sequence)
        ]
        if "exitSequence" in case:
            exit_sequence = case["exitSequence"]
            if (
                not isinstance(exit_sequence, list)
                or len(exit_sequence) != len(sequence)
                or any(
                    not isinstance(value, int)
                    or isinstance(value, bool)
                    or not 0 <= value <= 255
                    for value in exit_sequence
                )
                or exit_sequence[-1] != expected["exit"]
            ):
                fail(f"invalid exitSequence: {case_id}")
        else:
            exit_sequence = [0 for _ in sequence]
            exit_sequence[-1] = expected["exit"]

    with tempfile.TemporaryDirectory(prefix=f"pr72-lifecycle-{case_id}-") as temporary:
        temp = pathlib.Path(temporary)
        state_root = temp / "state"
        work = temp / "work"
        home = temp / "home"
        for directory in (state_root, work, home):
            directory.mkdir(mode=0o700)
        materialize_files(work, case["files"])
        if case_id == "preflight-state-invalid":
            # R28 requires a real state-store failure from a fresh empty root.
            # An invalid final-component mode trips the production 0700 check
            # without injecting any journal, session, or projection record.
            os.chmod(state_root, 0o500)
        script_path = temp / "providerScript.resolved.json"
        script_length = resolve_provider_script(
            case_dir, case["providerScript"], script_path
        )
        environment = {
            "HOME": str(home),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "PATH": os.pathsep.join(
                [
                    str(fake_provider.parent),
                    os.environ.get("PATH", "/usr/bin:/bin"),
                ]
            ),
            "PYTHONDONTWRITEBYTECODE": "1",
            "GPT_WEBAI_FAKE_SCRIPT": str(script_path),
            "GPT_WEBAI_STATE_ROOT": str(state_root),
        }
        environment.update(case_env)
        envelopes = []
        contributed_event_ids = []
        contributed_receipt_ids = []
        for index, argv in enumerate(sequence):
            step_environment = dict(environment)
            step_environment.update(env_sequence[index])
            step_failpoint = env_sequence[index].get("GPT_WEBAI_FAILPOINT")
            expected_exit = exit_sequence[index]
            if (expected_exit == 99) != (step_failpoint is not None):
                fail(
                    f"{case_id} command {index} failpoint/exitSequence disagreement"
                )
            before_head = read_head_event_id(state_root)
            before_receipts = receipt_index(state_root)
            completed = subprocess.run(
                [str(binary), *command_argv(argv)],
                cwd=work,
                env=step_environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=45,
                check=False,
            )
            exit_code = completed.returncode if completed.returncode >= 0 else 128 - completed.returncode
            if exit_code != expected_exit:
                fail(
                    f"{case_id} command {index} exit={exit_code} expected={expected_exit} "
                    f"stdout={completed.stdout.decode('utf-8', 'replace')[:1000]} "
                    f"stderr={completed.stderr.decode('utf-8', 'replace')[:1000]}"
                )
            after_head = read_head_event_id(state_root)
            after_receipts = receipt_index(state_root)
            if expected_exit == 99:
                if completed.stdout:
                    fail(f"{case_id} command {index} failpoint stdout is not empty")
                expected_stderr = f"failpoint:{step_failpoint}\n".encode("utf-8")
                if completed.stderr != expected_stderr:
                    fail(f"{case_id} command {index} failpoint stderr is not exact")
                if after_receipts != before_receipts:
                    fail(f"{case_id} command {index} failpoint created a receipt")
                contributed_event_ids.extend(
                    ordered_head_segment(state_root, before_head, after_head)
                )
                continue

            try:
                envelope = json.loads(completed.stdout)
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                fail(
                    f"{case_id} command {index} stdout is not one JSON object: {error} "
                    f"exit={exit_code} "
                    f"stdout={completed.stdout.decode('utf-8', 'replace')[:1000]} "
                    f"stderr={completed.stderr.decode('utf-8', 'replace')[:1000]}"
                )
            if completed.stdout != canonical_bytes(envelope):
                fail(f"{case_id} command {index} stdout is not canonical JSON plus LF")
            if envelope.get("schema") != "gpt-webai.lifecycle.r13.v1":
                fail(f"{case_id} command {index} emitted a non-R13 envelope")
            envelopes.append(envelope)
            contributed_event_ids.extend(envelope["eventIds"])
            contributed_receipt_ids.extend(envelope["receiptIds"])

        if not envelopes or exit_sequence[-1] == 99:
            fail(f"{case_id} final step must emit the expected envelope")
        final = envelopes[-1]
        for field in ("resultKind", "ok", "terminal", "reason"):
            if final.get(field) != expected[field]:
                fail(
                    f"{case_id} {field}={final.get(field)!r} expected={expected[field]!r}"
                )
        event_ids = contributed_event_ids
        receipt_ids = contributed_receipt_ids
        if len(event_ids) != len(set(event_ids)):
            fail(f"{case_id} emitted duplicate event IDs")
        event_types = []
        for event_id in event_ids:
            event = read_canonical_json(state_root / "journal/events" / f"{event_id}.json")
            if event.get("eventId") != event_id or not isinstance(event.get("eventType"), str):
                fail(f"{case_id} event identity mismatch: {event_id}")
            event_types.append(event["eventType"])
        receipts = receipt_index(state_root)
        try:
            receipt_ops = [receipts[receipt_id] for receipt_id in receipt_ids]
        except KeyError as error:
            fail(f"{case_id} receipt file is missing: {error.args[0]}")
        if event_types != expected["eventTypes"]:
            fail(
                f"{case_id} eventTypes mismatch\nactual={event_types!r}\n"
                f"expected={expected['eventTypes']!r}"
            )
        if receipt_ops != expected["receiptOps"]:
            fail(
                f"{case_id} receiptOps mismatch\nactual={receipt_ops!r}\n"
                f"expected={expected['receiptOps']!r}"
            )
        if bool(event_ids) != expected["mutates"]:
            fail(f"{case_id} mutates oracle disagrees with emitted events")
        counter_path = pathlib.Path(str(script_path) + ".counter")
        consumed = int(counter_path.read_text(encoding="ascii")) if counter_path.exists() else 0
        if consumed != script_length:
            fail(
                f"{case_id} consumed {consumed} provider frames; expected {script_length}"
            )
        if case_id == "preflight-state-invalid":
            os.chmod(state_root, 0o700)
    print(f"PASS lifecycle-r13 {case_id}")
    return case_id


if not fixture_root.is_dir() or fixture_root.is_symlink():
    fail("fixture root is missing or unsafe")
case_paths = sorted(fixture_root.glob("*/case.json"), key=lambda path: path.as_posix())
if not case_paths:
    fail("no lifecycle-r13 case.json files found")
case_ids = []
for case_path in case_paths:
    case_id = run_case(case_path)
    if case_id in case_ids:
        fail(f"duplicate caseId: {case_id}")
    case_ids.append(case_id)
missing = sorted(required_cases - set(case_ids))
if missing:
    fail("missing required explicit cases: " + ", ".join(missing))
print(f"PASS lifecycle-r13 fixtures={len(case_ids)}")
PY
}

run_fake_only() {
  assert_fake_paths
  (
    cd "$stack_root"
    python3 scripts/check-cli-fixtures-r13.py \
      --accepted tests/fixtures/cli-r13/accepted.jsonl \
      --rejected tests/fixtures/cli-r13/rejected.jsonl \
      --binary target/debug/gpt-webai-lifecycle
    run_lifecycle_fixture_replay
  )
}

run_full_only() {
  assert_full_paths
  command -v docker >/dev/null 2>&1 || fail 'docker is required for the full gate'
  docker info >/dev/null 2>&1 || fail 'docker daemon is unavailable for the full gate'

  (
    set -euo pipefail
    local full_root suffix project container_prefix container image docker_wrapper
    local real_docker before_slots after_slots run_id request_id prompt attachment
    full_root="$(mktemp -d "${TMPDIR:-/tmp}/gpt-webai-r13-full.XXXXXX")"
    chmod 0700 "$full_root"
    suffix="$(printf '%s' "$$-$(date +%s%N)" | sha256sum | cut -c1-12)"
    project="pr72-r13-full-$suffix"
    container_prefix="pr72-r13-full-$suffix"
    container="$container_prefix-slot-01"
    image="home-server/gpt-webai-slot-r13-fake:$suffix"
    real_docker="$(command -v docker)"
    docker_wrapper="$full_root/docker"
    request_id="request-full-$suffix"
    run_id="run-full-$suffix"
    prompt="$full_root/prompt.txt"
    attachment="$full_root/file-A.txt"

    cleanup_full_gate() {
      local cleanup_rc=$?
      set +e
      "$real_docker" compose -p "$project" \
        -f "$stack_root/compose.yaml" -f "$stack_root/compose.fake.yaml" \
        down --volumes --remove-orphans >/dev/null 2>&1
      "$real_docker" image rm "$image" >/dev/null 2>&1
      if [[ -n "$full_root" && -d "$full_root" ]]; then
        rm -rf -- "$full_root"
      fi
      return "$cleanup_rc"
    }
    trap cleanup_full_gate EXIT INT TERM

    install -d -m 0700 "$full_root/state"
    python3 - "$prompt" "$attachment" "$docker_wrapper" "$real_docker" "$project" <<'PY'
import os
import pathlib
import shlex
import sys

prompt = pathlib.Path(sys.argv[1])
attachment = pathlib.Path(sys.argv[2])
wrapper = pathlib.Path(sys.argv[3])
real_docker = shlex.quote(sys.argv[4])
fake_project = shlex.quote(sys.argv[5])
prompt.write_bytes(b"PR72 containerized fake R13 round trip\n")
attachment.write_bytes((b"PR72-FULL-FILE-A\n" * 4)[:64])
wrapper.write_text(
    """#!/usr/bin/env bash
set -euo pipefail
real_docker=__REAL_DOCKER__
fake_project=__FAKE_PROJECT__
args=(\"$@\")
if [[ ${args[0]:-} == compose ]]; then
  for ((index=1; index < ${#args[@]} - 1; index++)); do
    if [[ ${args[index]} == -p && ${args[index + 1]} == gpt-webai-slot-pool ]]; then
      args[index + 1]=$fake_project
    fi
  done
fi
exec \"$real_docker\" \"${args[@]}\"
""".replace("__REAL_DOCKER__", real_docker).replace(
        "__FAKE_PROJECT__", fake_project
    ),
    encoding="utf-8",
    newline="\n",
)
for path, mode in ((prompt, 0o600), (attachment, 0o600), (wrapper, 0o700)):
    os.chmod(path, mode)
PY

    export GPT_WEBAI_STATE_ROOT="$full_root/state"
    export GPT_WEBAI_SLOT_COUNT=1
    export GPT_WEBAI_SLOT_MODE=docker
    export GPT_WEBAI_SLOT_CONTAINER_PREFIX="$container_prefix-"
    export GPT_WEBAI_RUST_STATUS_PROVIDER_CHECK=false
    export GPT_WEBAI_SLOT_UID="$(id -u)"
    export GPT_WEBAI_SLOT_GID="$(id -g)"
    export GPT_WEBAI_FAKE_IMAGE="$image"
    export GPT_WEBAI_FAKE_PROVIDER_PATH="$stack_root/tests/gpt-webai-lifecycle/fixtures/fake-bin/gpt-webai-provider"
    export GPT_WEBAI_FAKE_CONTAINER_PREFIX="$container_prefix"
    export COMPOSE_FILE="$stack_root/compose.yaml:$stack_root/compose.fake.yaml"
    export PR72_OWNER_ID="owner_$(printf '%s' "$suffix-owner" | sha256sum | cut -d' ' -f1)"
    export PR72_OWNER_GENERATION=1
    export PR72_RUNTIME_INCARNATION="runtime_$(printf '%s' "$suffix-runtime" | sha256sum | cut -d' ' -f1)"

    before_slots="$($real_docker ps -a --filter 'name=^/gpt-webai-slot-' \
      --format '{{.Names}}\t{{.Status}}\t{{.ID}}' | LC_ALL=C sort)"

    "$real_docker" compose -p "$project" \
      -f "$stack_root/compose.yaml" -f "$stack_root/compose.fake.yaml" \
      config --format json >"$full_root/compose.rendered.json"
    python3 - "$full_root/compose.rendered.json" "$GPT_WEBAI_STATE_ROOT" \
      "$GPT_WEBAI_FAKE_PROVIDER_PATH" "$container" "$image" <<'PY'
import json
import pathlib
import sys

document = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
state_root, fake_provider, container, image = sys.argv[2:]
service = document["services"]["gpt-webai-slot-01"]
assert service["container_name"] == container
assert service["image"] == image
assert service["network_mode"] == "none"
assert service["restart"] == "no"
labels = service["labels"]
assert set(labels) == {
    "pr72.gpt-webai.owner-generation",
    "pr72.gpt-webai.owner-id",
    "pr72.gpt-webai.runtime-incarnation",
}
volumes = {item["target"]: item for item in service["volumes"]}
assert set(volumes) == {
    "/state/slot-01",
    "/broker-attachments",
    "/broker-prompts",
    "/broker-artifacts",
    "/usr/local/bin/node",
}
assert volumes["/state/slot-01"]["source"] == f"{state_root}/slots/slot-01/state"
assert volumes["/broker-attachments"].get("read_only") is True
assert volumes["/broker-prompts"].get("read_only") is True
assert volumes["/broker-artifacts"]["source"] == f"{state_root}/artifacts"
assert volumes["/usr/local/bin/node"]["source"] == fake_provider
assert volumes["/usr/local/bin/node"].get("read_only") is True
PY

    "$real_docker" compose -p "$project" \
      -f "$stack_root/compose.yaml" -f "$stack_root/compose.fake.yaml" \
      build gpt-webai-slot-01

    if ! "$stack_root/target/debug/gpt-webai-lifecycle" run \
      --json --docker-slot-provider --live-send --require-visual-gate \
      --docker-bin "$docker_wrapper" \
      --request-id "$request_id" --run-id "$run_id" \
      --fencing-token "fence-$suffix" --model pro --effort standard \
      --prompt-file "$prompt" --file "$attachment" \
      --artifact-expectation optional --provider-timeout-ms 500000 \
      --runtime-start-timeout-ms 120000 --runtime-stop-timeout-ms 120000 \
      >"$full_root/run.stdout" 2>"$full_root/run.stderr"; then
      sed -n '1,240p' "$full_root/run.stderr" >&2
      fail 'containerized fake lifecycle round trip failed'
    fi
    python3 - "$full_root/run.stdout" "$GPT_WEBAI_STATE_ROOT" "$request_id" <<'PY'
import json
import pathlib
import sys

stdout = pathlib.Path(sys.argv[1]).read_bytes()
value = json.loads(stdout)
assert stdout.endswith(b"\n") and stdout.count(b"\n") == 1
assert value["schema"] == "gpt-webai.lifecycle.r13.v1"
assert value["command"] == "run"
assert value["resultKind"] == "run.terminal_optional_zero"
assert value["ok"] is True and value["terminal"] is True
assert value["sessionId"] == "sid-cli-docker"
assert value["conversationUrl"] == "https://chatgpt.com/c/sid-cli-docker"
assert not any("WEB:" in str(item) for item in value.values())
state_root = pathlib.Path(sys.argv[2])
request_id = sys.argv[3]
assert (state_root / "sessions/sid-cli-docker.json").is_file()
assert (state_root / f"artifacts/r-{request_id}").is_dir()
assert value["eventIds"] and value["receiptIds"]
PY

    [[ "$($real_docker inspect --format '{{.HostConfig.NetworkMode}}' "$container")" == none ]] ||
      fail 'full-gate container is not network-isolated'
    [[ "$($real_docker inspect --format '{{.State.Status}}' "$container")" == exited ]] ||
      fail 'lifecycle did not stop the full-gate container'
    "$real_docker" inspect --format '{{json .Config.Labels}}' "$container" |
      python3 -c 'import json,re,sys; v=json.load(sys.stdin); assert re.fullmatch(r"owner_[0-9a-f]{64}",v["pr72.gpt-webai.owner-id"]); assert re.fullmatch(r"[1-9][0-9]{0,4}",v["pr72.gpt-webai.owner-generation"]); assert re.fullmatch(r"runtime_[0-9a-f]{64}",v["pr72.gpt-webai.runtime-incarnation"])'

    "$real_docker" start "$container" >/dev/null
    healthy=0
    for _ in $(seq 1 120); do
      if [[ "$($real_docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container")" == healthy ]]; then
        healthy=1
        break
      fi
      sleep 0.5
    done
    [[ "$healthy" == 1 ]] || fail 'full-gate container did not become healthy'
    "$real_docker" exec "$container" /bin/sh -eu -c \
      "command -v python3 >/dev/null; test -r /broker-prompts/$run_id/prompt.txt; test -n \"\$(find /broker-attachments/$run_id -type f -print -quit)\"; test -n \"\$(find /state/slot-01/evidence -name provider-request.json -print -quit)\"; test -n \"\$(find /broker-artifacts/r-$request_id -type f -print -quit)\""
    if "$real_docker" exec "$container" /bin/sh -c \
      'printf x > /broker-prompts/pr72-write-probe' >/dev/null 2>&1; then
      fail 'prompt mount is writable in the full-gate container'
    fi
    if "$real_docker" exec "$container" /bin/sh -c \
      'printf x > /broker-attachments/pr72-write-probe' >/dev/null 2>&1; then
      fail 'attachment mount is writable in the full-gate container'
    fi
    "$real_docker" stop "$container" >/dev/null

    status_output="$($stack_root/target/debug/gpt-webai-lifecycle status --legacy-kv)"
    grep -qx 'holders=0' <<<"$status_output"
    grep -qx 'locks=0' <<<"$status_output"
    after_slots="$($real_docker ps -a --filter 'name=^/gpt-webai-slot-' \
      --format '{{.Names}}\t{{.Status}}\t{{.ID}}' | LC_ALL=C sort)"
    [[ "$before_slots" == "$after_slots" ]] ||
      fail 'full gate changed a pre-existing production slot container'
    printf 'PASS full containerized fake R13 round trip container=%s\n' "$container"
  )
}

run_smoke_gate() {
  [[ "${GPT_WEBAI_LIVE:-}" == 1 ]] || usage_error
  require_executable scripts/live-smoke.sh
  exec bash "$stack_root/scripts/live-smoke.sh"
}

[[ "$#" -eq 1 ]] || usage_error
case "$1" in
  static) run_static_gate ;;
  fake)
    run_static_gate
    run_fake_only
    ;;
  full)
    run_static_gate
    run_fake_only
    run_full_only
    ;;
  smoke) run_smoke_gate ;;
  all)
    run_static_gate
    run_fake_only
    run_full_only
    ;;
  *) usage_error ;;
esac
