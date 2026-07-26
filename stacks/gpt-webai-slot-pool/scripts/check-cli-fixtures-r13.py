#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

SCHEMA = "pr72.cli-fixture.r13.v1"
TOP_KEYS = {"argv", "cwd", "env", "expected", "fixtureId", "kind", "schemaVersion", "stdinB64", "target"}
EXPECTED_KEYS = {
    "envelopeSubset", "exit", "mutates", "postStateDigest", "stderrContains",
    "stdoutKind", "stdoutSha256",
}
STDOUT_KINDS = {"lifecycle_envelope", "wrapper_envelope", "bytes", "empty"}
TARGETS = {"gptpro", "gptxhigh", "lifecycle"}
PROJECTION_ORDER = (
    "requests", "sessions", "slots", "allocator", "claims", "leases",
    "runtime_owners", "artifact_claims", "releases", "qa_counters",
)
H256 = re.compile(r"^sha256:[0-9a-f]{64}$")
WRAPPER_REQUIRED_KEYS = {
    "hardFailure", "networkDisconnected", "ok", "status", "usageError",
}
WRAPPER_STUB_ENVELOPE = {
    "hardFailure": False,
    "networkDisconnected": False,
    "ok": True,
    "status": "done",
    "usageError": False,
}
LIFECYCLE_KEYS = {
    "answerPath", "answerSha256", "answerSizeBytes", "answerText",
    "artifactClaims", "claimId", "cohort", "command", "conversationUrl",
    "evidenceRoot", "eventIds", "leaseId", "message", "ok", "operationId",
    "reason", "receiptIds", "requestId", "resultKind", "retry", "runId",
    "runtimeOwnerId", "schema", "sessionId", "slotId", "status", "terminal",
}
RETRY_KEYS = {"budget", "delayMs", "owner", "retryable"}
ARTIFACT_CLAIM_KEYS = {
    "artifactClaimId", "artifactIds", "expectation", "result", "status",
}
LIFECYCLE_RESULTS = {
    "status": {
        "status.ready", "status.blocked", "status.degraded", "status.state_invalid",
        "status.runtime_probe_failed", "status.lock_contended",
    },
    "preflight": {
        "preflight.ready", "preflight.model_correction_required",
        "preflight.login_required", "preflight.subscription_required",
        "preflight.provider_limit", "preflight.unreachable", "preflight.schema_drift",
        "preflight.no_slot", "preflight.state_invalid", "preflight.lock_contended",
    },
    "run": {
        "run.running", "run.terminal_success", "run.terminal_optional_zero",
        "run.queued_pool_busy", "run.model_failed", "run.upload_failed",
        "run.send_failed", "run.send_uncertain", "run.poll_failed",
        "run.artifact_required_failed", "run.output_publish_failed",
        "run.slot_readiness_failed", "run.release_failed", "run.lock_contended",
    },
    "show": {
        "show.running", "show.terminal", "show.idle", "show.unknown_session",
        "show.pinned_slot_unavailable", "show.url_rejected", "show.content_unavailable",
        "show.claim_conflict", "show.request_binding_missing", "show.provider_blocked",
        "show.release_failed", "show.lock_contended",
    },
    "resume": {
        "resume.running", "resume.terminal_success", "resume.terminal_optional_zero",
        "resume.unknown_session", "resume.pinned_slot_unavailable", "resume.url_rejected",
        "resume.content_unavailable", "resume.claim_conflict",
        "resume.output_publish_failed", "resume.request_binding_missing",
        "resume.provider_blocked", "resume.poll_failed",
        "resume.artifact_required_failed", "resume.release_failed",
        "resume.lock_contended",
    },
    "download": {
        "download.completed", "download.optional_zero", "download.unknown_session",
        "download.pinned_slot_unavailable", "download.url_rejected",
        "download.claim_conflict", "download.content_unavailable",
        "download.controls_absent_required", "download.ambiguous_controls",
        "download.event_timeout", "download.integrity_failed",
        "download.provider_blocked", "download.release_failed", "download.lock_contended",
    },
    "release": {
        "release.allocatable", "release.cooldown_blocked", "release.already_released",
        "release.stop_skipped_owner_alive", "release.target_unknown",
        "release.fencing_mismatch", "release.takeover_unproven", "release.stop_failed",
        "release.cleanup_failed", "release.lock_contended",
    },
    "cleanup": {
        "cleanup.plan", "cleanup.applied", "cleanup.state_invalid", "cleanup.unsafe_path",
        "cleanup.partial_failure", "cleanup.lock_contended",
    },
    "state-rebuild": {
        "state_rebuild.match", "state_rebuild.head_stale",
        "state_rebuild.snapshot_ignored", "state_rebuild.event_invalid",
        "state_rebuild.transition_invalid", "state_rebuild.digest_mismatch",
        "state_rebuild.lock_contended",
    },
    "allocate": {
        "allocate.dry_run_candidate", "allocate.pool_busy", "allocate.state_invalid",
        "allocate.lock_contended",
    },
}
NO_REASON_RESULTS = {
    "status.ready", "status.blocked", "status.degraded", "preflight.ready",
    "preflight.model_correction_required", "run.running", "run.terminal_success",
    "run.terminal_optional_zero", "run.queued_pool_busy", "show.running",
    "show.terminal", "show.idle", "resume.running", "resume.terminal_success",
    "resume.terminal_optional_zero", "download.completed", "download.optional_zero",
    "release.allocatable", "release.cooldown_blocked", "release.already_released",
    "release.stop_skipped_owner_alive", "cleanup.plan", "cleanup.applied",
    "state_rebuild.match", "state_rebuild.head_stale", "state_rebuild.snapshot_ignored",
    "allocate.dry_run_candidate", "allocate.pool_busy",
}
NONTERMINAL_RESULTS = {
    "run.running", "run.queued_pool_busy", "show.running", "show.idle", "resume.running",
}
OK_WITH_REASON_RESULTS = {
    "preflight.login_required", "preflight.subscription_required", "preflight.provider_limit",
}


class FixtureError(RuntimeError):
    pass


def strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise FixtureError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def canonical_line(value: Any) -> bytes:
    return (
        json.dumps(
            value,
            allow_nan=False,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        + "\n"
    ).encode()


def reject_non_json_constant(value: str) -> None:
    raise FixtureError(f"non-JSON numeric constant: {value}")


def exact_keys(value: dict[str, Any], expected: set[str], field: str) -> None:
    if set(value) != expected:
        raise FixtureError(f"{field} keys differ: expected={sorted(expected)} actual={sorted(value)}")


def nonempty(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise FixtureError(f"{field} must be non-empty text")
    return value


def validate_fixture(value: Any, source_kind: str, location: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise FixtureError(f"{location}: fixture must be an object")
    exact_keys(value, TOP_KEYS, f"{location} fixture")
    if value["schemaVersion"] != SCHEMA:
        raise FixtureError(f"{location}: wrong schemaVersion")
    if value["kind"] != source_kind or source_kind not in {"accepted", "rejected"}:
        raise FixtureError(f"{location}: kind/file mismatch")
    nonempty(value["fixtureId"], f"{location}.fixtureId")
    if value["target"] not in TARGETS:
        raise FixtureError(f"{location}: invalid target")
    argv = value["argv"]
    if not isinstance(argv, list) or len(argv) > 64 or any(not isinstance(item, str) for item in argv):
        raise FixtureError(f"{location}: argv must contain 0..64 strings")
    if not isinstance(value["stdinB64"], str):
        raise FixtureError(f"{location}: stdinB64 must be a string")
    try:
        base64.b64decode(value["stdinB64"], validate=True)
    except (TypeError, binascii.Error) as error:
        raise FixtureError(f"{location}: stdinB64 is not strict base64") from error
    env = value["env"]
    if not isinstance(env, dict) or any(not isinstance(key, str) or not isinstance(item, str) for key, item in env.items()):
        raise FixtureError(f"{location}: env must be a string map")
    if list(env) != sorted(env):
        raise FixtureError(f"{location}: env keys are not sorted")
    if value["cwd"] not in {"repo_root", "stack_root"}:
        raise FixtureError(f"{location}: invalid cwd")
    expected = value["expected"]
    if not isinstance(expected, dict):
        raise FixtureError(f"{location}: expected must be an object")
    exact_keys(expected, EXPECTED_KEYS, f"{location}.expected")
    if not isinstance(expected["exit"], int) or isinstance(expected["exit"], bool) or not 0 <= expected["exit"] <= 255:
        raise FixtureError(f"{location}: invalid exit")
    stdout_kind = expected["stdoutKind"]
    if stdout_kind not in STDOUT_KINDS:
        raise FixtureError(f"{location}: invalid stdoutKind")
    digest = expected["stdoutSha256"]
    if stdout_kind == "bytes":
        if not isinstance(digest, str) or H256.fullmatch(digest) is None:
            raise FixtureError(f"{location}: stdoutSha256 nullability")
    elif digest is not None:
        raise FixtureError(f"{location}: stdoutSha256 nullability")
    subset = expected["envelopeSubset"]
    if stdout_kind.endswith("_envelope"):
        if not isinstance(subset, dict):
            raise FixtureError(f"{location}: envelopeSubset nullability")
        required_subset = (
            {"resultKind", "status", "ok", "terminal", "reason"}
            if stdout_kind == "lifecycle_envelope"
            else WRAPPER_REQUIRED_KEYS
        )
        if not required_subset.issubset(subset):
            missing = sorted(required_subset - set(subset))
            raise FixtureError(
                f"{location}: envelopeSubset missing minimum fields: {', '.join(missing)}"
            )
        if stdout_kind == "lifecycle_envelope" and value["target"] != "lifecycle":
            raise FixtureError(f"{location}: lifecycle envelope requires lifecycle target")
        if stdout_kind == "wrapper_envelope" and value["target"] == "lifecycle":
            raise FixtureError(f"{location}: wrapper envelope requires wrapper target")
    elif subset is not None:
        raise FixtureError(f"{location}: envelopeSubset nullability")
    stderr_contains = expected["stderrContains"]
    if stderr_contains is not None and not isinstance(stderr_contains, str):
        raise FixtureError(f"{location}: stderrContains must be text or null")
    if not isinstance(expected["mutates"], bool):
        raise FixtureError(f"{location}: mutates must be boolean")
    state_digest = expected["postStateDigest"]
    if expected["mutates"]:
        if not isinstance(state_digest, str) or H256.fullmatch(state_digest) is None:
            raise FixtureError(f"{location}: postStateDigest nullability")
    elif state_digest is not None:
        raise FixtureError(f"{location}: postStateDigest nullability")
    return value


def load_jsonl(path: Path, kind: str) -> list[dict[str, Any]]:
    if not path.is_file() or path.is_symlink():
        raise FixtureError(f"missing or unsafe fixture file: {path}")
    data = path.read_bytes()
    if not data or not data.endswith(b"\n") or b"\r" in data or b"\x00" in data:
        raise FixtureError(f"invalid JSONL serialization: {path}")
    result = []
    for index, raw in enumerate(data.splitlines(keepends=True), start=1):
        if raw == b"\n":
            raise FixtureError(f"blank fixture line: {path}:{index}")
        try:
            value = json.loads(
                raw,
                object_pairs_hook=strict_object,
                parse_constant=reject_non_json_constant,
            )
        except (UnicodeDecodeError, json.JSONDecodeError, FixtureError) as error:
            raise FixtureError(f"invalid fixture JSON: {path}:{index}: {error}") from error
        if canonical_line(value) != raw:
            raise FixtureError(f"non-canonical fixture JSON: {path}:{index}")
        result.append(validate_fixture(value, kind, f"{path}:{index}"))
    return result


def tree_digest(root: Path) -> str:
    lines: list[bytes] = []
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix().encode()):
        info = path.lstat()
        rel = path.relative_to(root).as_posix()
        if stat.S_ISREG(info.st_mode):
            payload = hashlib.sha256(path.read_bytes()).hexdigest()
            kind = "file"
        elif stat.S_ISDIR(info.st_mode):
            continue
        elif stat.S_ISLNK(info.st_mode):
            payload = hashlib.sha256(os.readlink(path).encode()).hexdigest()
            kind = "symlink"
        else:
            payload = "-"
            kind = "other"
        lines.append(f"{kind}\t{info.st_mode & 0o7777:o}\t{payload}\t{rel}\n".encode())
    return "sha256:" + hashlib.sha256(b"".join(lines)).hexdigest()


def projection_digest(root: Path) -> str:
    directory = root / "journal" / "projections"
    chunks = []
    for name in PROJECTION_ORDER:
        path = directory / f"{name}.json"
        if not path.is_file() or path.is_symlink():
            raise FixtureError(f"missing projection for mutating fixture: {path}")
        chunks.append(path.read_bytes())
    return "sha256:" + hashlib.sha256(b"".join(chunks)).hexdigest()


def subset_matches(actual: Any, expected: Any) -> bool:
    if isinstance(expected, dict):
        return isinstance(actual, dict) and all(
            key in actual and subset_matches(actual[key], value) for key, value in expected.items()
        )
    return actual == expected


def optional_text(value: Any, field: str) -> None:
    if value is not None:
        nonempty(value, field)


def string_list(value: Any, field: str, pattern: re.Pattern[str] | None = None) -> None:
    if not isinstance(value, list) or len(value) > 64 or len(set(value)) != len(value):
        raise FixtureError(f"{field} must be a duplicate-free 0..64 string list")
    for index, item in enumerate(value):
        nonempty(item, f"{field}[{index}]")
        if pattern is not None and pattern.fullmatch(item) is None:
            raise FixtureError(f"{field}[{index}] has an invalid identifier")


def validate_lifecycle_envelope(value: Any, fixture_id: str) -> None:
    if not isinstance(value, dict):
        raise FixtureError(f"{fixture_id}: lifecycle envelope must be an object")
    exact_keys(value, LIFECYCLE_KEYS, f"{fixture_id} lifecycle envelope")
    if value["schema"] != "gpt-webai.lifecycle.r13.v1":
        raise FixtureError(f"{fixture_id}: wrong lifecycle schema")
    command = value["command"]
    result_kind = value["resultKind"]
    if command not in LIFECYCLE_RESULTS or result_kind not in LIFECYCLE_RESULTS[command]:
        raise FixtureError(f"{fixture_id}: unknown lifecycle command/result pair")
    if value["status"] != result_kind:
        raise FixtureError(f"{fixture_id}: status must equal resultKind")
    nonempty(value["operationId"], f"{fixture_id}.operationId")
    nonempty(value["message"], f"{fixture_id}.message")
    if not isinstance(value["ok"], bool) or not isinstance(value["terminal"], bool):
        raise FixtureError(f"{fixture_id}: ok/terminal must be boolean")
    expected_terminal = result_kind not in NONTERMINAL_RESULTS
    expected_ok = result_kind in NO_REASON_RESULTS or result_kind in OK_WITH_REASON_RESULTS
    if value["terminal"] != expected_terminal or value["ok"] != expected_ok:
        raise FixtureError(f"{fixture_id}: ok/terminal disagree with the result matrix")
    reason = value["reason"]
    if (result_kind in NO_REASON_RESULTS) != (reason is None):
        raise FixtureError(f"{fixture_id}: reason presence disagrees with the result matrix")
    optional_text(reason, f"{fixture_id}.reason")

    retry = value["retry"]
    if not isinstance(retry, dict):
        raise FixtureError(f"{fixture_id}: retry must be an object")
    exact_keys(retry, RETRY_KEYS, f"{fixture_id}.retry")
    if (
        not isinstance(retry["budget"], int) or isinstance(retry["budget"], bool)
        or not 0 <= retry["budget"] <= 65535
        or not isinstance(retry["delayMs"], int) or isinstance(retry["delayMs"], bool)
        or retry["delayMs"] < 0
        or not isinstance(retry["retryable"], bool)
    ):
        raise FixtureError(f"{fixture_id}: invalid retry fields")
    optional_text(retry["owner"], f"{fixture_id}.retry.owner")
    if retry["retryable"]:
        if retry["owner"] is None or retry["delayMs"] == 0:
            raise FixtureError(f"{fixture_id}: retryable result lacks owner/delay")
    elif retry != {"budget": 0, "delayMs": 0, "owner": None, "retryable": False}:
        raise FixtureError(f"{fixture_id}: non-retryable tuple must be none/0/0")

    for field in (
        "answerPath", "answerSha256", "answerText", "claimId", "cohort",
        "conversationUrl", "evidenceRoot", "leaseId", "requestId", "runId",
        "runtimeOwnerId", "sessionId", "slotId",
    ):
        optional_text(value[field], f"{fixture_id}.{field}")
    answer_tuple = (value["answerPath"], value["answerSha256"], value["answerSizeBytes"])
    if any(item is not None for item in answer_tuple) != all(item is not None for item in answer_tuple):
        raise FixtureError(f"{fixture_id}: answer path/hash/size must be all-null or all-present")
    if value["answerSha256"] is not None and H256.fullmatch(value["answerSha256"]) is None:
        raise FixtureError(f"{fixture_id}: invalid answerSha256")
    if value["answerSizeBytes"] is not None and (
        not isinstance(value["answerSizeBytes"], int)
        or isinstance(value["answerSizeBytes"], bool)
        or value["answerSizeBytes"] <= 0
    ):
        raise FixtureError(f"{fixture_id}: invalid answerSizeBytes")
    if value["answerText"] is not None and len(value["answerText"].encode()) > 65536:
        raise FixtureError(f"{fixture_id}: answerText exceeds 65536 bytes")

    string_list(value["eventIds"], f"{fixture_id}.eventIds", re.compile(r"^evt_[0-9a-f]{64}$"))
    string_list(
        value["receiptIds"], f"{fixture_id}.receiptIds", re.compile(r"^receipt_[0-9a-f]{64}$")
    )
    claims = value["artifactClaims"]
    if not isinstance(claims, list) or len(claims) > 64:
        raise FixtureError(f"{fixture_id}: artifactClaims must contain 0..64 objects")
    for index, claim in enumerate(claims):
        if not isinstance(claim, dict):
            raise FixtureError(f"{fixture_id}: artifactClaims[{index}] must be an object")
        exact_keys(claim, ARTIFACT_CLAIM_KEYS, f"{fixture_id}.artifactClaims[{index}]")
        nonempty(claim["artifactClaimId"], f"{fixture_id}.artifactClaims[{index}].artifactClaimId")
        if claim["expectation"] not in {"optional", "required", "claimed"}:
            raise FixtureError(f"{fixture_id}: invalid artifact expectation")
        if claim["status"] not in {"established", "completed", "failed"}:
            raise FixtureError(f"{fixture_id}: invalid artifact status")
        optional_text(claim["result"], f"{fixture_id}.artifactClaims[{index}].result")
        if (claim["status"] == "completed") != (claim["result"] is not None):
            raise FixtureError(f"{fixture_id}: artifact result nullability")
        string_list(claim["artifactIds"], f"{fixture_id}.artifactClaims[{index}].artifactIds")


def parse_envelope(stdout: bytes, fixture_id: str, require_canonical: bool) -> Any:
    if not stdout.endswith(b"\n") or stdout.count(b"\n") != 1:
        raise FixtureError(f"{fixture_id}: envelope stdout is not one LF-terminated object")
    try:
        value = json.loads(
            stdout,
            object_pairs_hook=strict_object,
            parse_constant=reject_non_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, FixtureError) as error:
        raise FixtureError(f"{fixture_id}: invalid envelope JSON: {error}") from error
    if require_canonical and canonical_line(value) != stdout:
        raise FixtureError(f"{fixture_id}: lifecycle envelope is not canonical JSON")
    return value


def write_recording_stub(directory: Path, record_path: Path) -> Path:
    stub = directory / "gpt-webai-lifecycle"
    payload = json.dumps(WRAPPER_STUB_ENVELOPE, sort_keys=True, separators=(",", ":"))
    stub.write_text(
        "#!/usr/bin/env python3\n"
        "import json, os, sys\n"
        f"record_path = {str(record_path)!r}\n"
        "record = {\"argv\": sys.argv[1:], \"env\": dict(os.environ)}\n"
        "with open(record_path, \"x\", encoding=\"utf-8\", newline=\"\\n\") as handle:\n"
        "    json.dump(record, handle, sort_keys=True, separators=(\",\", \":\"))\n"
        "    handle.write(\"\\n\")\n"
        f"sys.stdout.write({payload!r} + \"\\n\")\n",
        encoding="utf-8",
        newline="\n",
    )
    stub.chmod(0o700)
    return stub


def wrapper_prompt_and_files(argv: list[str], stdin: bytes) -> tuple[str, list[str]]:
    files: list[str] = []
    prompt_parts: list[str] = []
    index = 0
    while index < len(argv):
        token = argv[index]
        if token == "--file":
            if index + 1 >= len(argv) or not argv[index + 1]:
                return "", files
            files.append(argv[index + 1])
            index += 2
        elif token.startswith("--file="):
            value = token.removeprefix("--file=")
            if not value:
                return "", files
            files.append(value)
            index += 1
        elif token == "--":
            prompt_parts.extend(argv[index + 1 :])
            break
        else:
            prompt_parts.append(token)
            index += 1
    prompt = " ".join(prompt_parts) if prompt_parts else stdin.decode("utf-8")
    return prompt.rstrip("\n"), files


def expected_wrapper_argv(
    fixture: dict[str, Any], stdin: bytes, environment: dict[str, str],
) -> list[str]:
    prompt, files = wrapper_prompt_and_files(fixture["argv"], stdin)
    if not prompt.strip():
        raise FixtureError(f"{fixture['fixtureId']}: accepted wrapper prompt is empty")
    home = Path(environment["HOME"])
    prelude_path = home / ".codex/prompts/gpt-delegation-prelude.md"
    if prelude_path.is_file() and os.access(prelude_path, os.R_OK):
        prelude = prelude_path.read_text(encoding="utf-8").rstrip("\n")
    else:
        prelude = (
            "You are receiving a delegated task. Do not downscope A into A1; return a "
            "complete, final, implementation-ready answer for the requested scope while "
            "preserving safety, authorization, secrets, destructive-action controls, test "
            "integrity, and user-change preservation."
        )
    rendered_prompt = f"{prelude}\n\nUSER TASK:\n{prompt}"
    kind = "pro" if fixture["target"] == "gptpro" else "xhigh"
    result = ["run", "--kind", kind]
    for path in files:
        result.extend(["--file", path])
    result.extend(["--prompt", rendered_prompt])
    return result


def validate_wrapper_recording(
    fixture: dict[str, Any], record_path: Path, stdin: bytes, environment: dict[str, str],
) -> None:
    if not record_path.is_file():
        raise FixtureError(f"{fixture['fixtureId']}: wrapper did not invoke the recording stub")
    try:
        record = json.loads(record_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise FixtureError(f"{fixture['fixtureId']}: invalid wrapper recording") from error
    if not isinstance(record, dict) or set(record) != {"argv", "env"}:
        raise FixtureError(f"{fixture['fixtureId']}: invalid wrapper recording keys")
    expected_argv = expected_wrapper_argv(fixture, stdin, environment)
    if record["argv"] != expected_argv:
        raise FixtureError(f"{fixture['fixtureId']}: wrapper argv shape mismatch")
    recorded_env = record["env"]
    if not isinstance(recorded_env, dict):
        raise FixtureError(f"{fixture['fixtureId']}: wrapper env recording is invalid")
    timeout_name = "GPTPRO_TIMEOUT" if fixture["target"] == "gptpro" else "GPTXHIGH_TIMEOUT"
    timeout_default = "10800" if fixture["target"] == "gptpro" else "300"
    expected_timeout = environment.get(timeout_name, timeout_default)
    for name, expected in {
        timeout_name: expected_timeout,
        "CHROME_NO_SANDBOX": environment.get("CHROME_NO_SANDBOX", "1"),
        "DISPLAY": environment["DISPLAY"],
        "CHROME_BINARY_PATH": environment["CHROME_BINARY_PATH"],
    }.items():
        if recorded_env.get(name) != expected:
            raise FixtureError(f"{fixture['fixtureId']}: wrapper env mismatch for {name}")


def execute_fixture(
    fixture: dict[str, Any], binary: Path, repo_root: Path, stack_root: Path,
) -> None:
    fixture_id = fixture["fixtureId"]
    expected = fixture["expected"]
    with tempfile.TemporaryDirectory(prefix=f"pr72-cli-{fixture_id}-") as directory:
        temp_root = Path(directory)
        state_root = temp_root / "state"
        state_root.mkdir(mode=0o700)
        before = tree_digest(state_root)
        target = fixture["target"]
        executable = binary if target == "lifecycle" else Path.home() / ".local" / "bin" / target
        if not executable.is_file() or not os.access(executable, os.X_OK):
            raise FixtureError(f"{fixture_id}: executable missing: {executable}")
        environment = dict(fixture["env"])
        empty_home = temp_root / "empty-home"
        empty_home.mkdir(mode=0o700)
        environment = {
            name: (
                str(empty_home)
                if value == "${FIXTURE_EMPTY_HOME}"
                else str(Path.home())
                if value == "${REAL_HOME}"
                else value
            )
            for name, value in environment.items()
        }
        environment.setdefault("HOME", str(Path.home()))
        environment.setdefault("PATH", os.environ.get("PATH", "/usr/bin:/bin"))
        environment["GPT_WEBAI_STATE_ROOT"] = str(state_root)
        stdin = base64.b64decode(fixture["stdinB64"], validate=True)
        record_path = temp_root / "wrapper-recording.json"
        if target != "lifecycle":
            stub_dir = temp_root / "recording-bin"
            stub_dir.mkdir(mode=0o700)
            write_recording_stub(stub_dir, record_path)
            environment["PATH"] = f"{stub_dir}:{environment['PATH']}"
        completed = subprocess.run(
            [str(executable), *fixture["argv"]],
            cwd=repo_root if fixture["cwd"] == "repo_root" else stack_root,
            env=environment,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != expected["exit"]:
            raise FixtureError(f"{fixture_id}: exit {completed.returncode}, expected {expected['exit']}")
        stdout_kind = expected["stdoutKind"]
        if stdout_kind == "empty" and completed.stdout:
            raise FixtureError(f"{fixture_id}: stdout must be empty")
        if stdout_kind == "bytes":
            observed = "sha256:" + hashlib.sha256(completed.stdout).hexdigest()
            if observed != expected["stdoutSha256"]:
                raise FixtureError(f"{fixture_id}: stdout SHA-256 mismatch")
        if stdout_kind.endswith("_envelope"):
            envelope = parse_envelope(
                completed.stdout,
                fixture_id,
                require_canonical=stdout_kind == "lifecycle_envelope",
            )
            if stdout_kind == "lifecycle_envelope":
                validate_lifecycle_envelope(envelope, fixture_id)
            if not subset_matches(envelope, expected["envelopeSubset"]):
                raise FixtureError(f"{fixture_id}: envelope subset mismatch")
        if target != "lifecycle" and fixture["kind"] == "accepted":
            validate_wrapper_recording(fixture, record_path, stdin, environment)
        needle = expected["stderrContains"]
        if needle is not None and needle.encode() not in completed.stderr:
            raise FixtureError(f"{fixture_id}: stderr does not contain expected bytes")
        after = tree_digest(state_root)
        if not expected["mutates"] and after != before:
            raise FixtureError(f"{fixture_id}: unexpected state mutation")
        if expected["mutates"] and projection_digest(state_root) != expected["postStateDigest"]:
            raise FixtureError(f"{fixture_id}: post-state projection digest mismatch")


def validate_coverage(fixtures: list[dict[str, Any]]) -> None:
    accepted_targets = {
        fixture["target"] for fixture in fixtures if fixture["kind"] == "accepted"
    }
    missing_targets = sorted(TARGETS - accepted_targets)
    if missing_targets:
        raise FixtureError(
            "accepted fixture coverage is missing targets: " + ", ".join(missing_targets)
        )

    rejected_commands: set[str] = set()
    for fixture in fixtures:
        if fixture["target"] != "lifecycle" or not fixture["argv"]:
            continue
        command = fixture["argv"][0]
        if fixture["kind"] == "rejected":
            rejected_commands.add(command)

    required_commands = set(LIFECYCLE_RESULTS)
    missing_rejected = sorted(required_commands - rejected_commands)
    if missing_rejected:
        raise FixtureError(
            "rejected grammar coverage is missing lifecycle commands: "
            + ", ".join(missing_rejected)
        )


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--accepted", required=True, type=Path)
    parser.add_argument("--rejected", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    script = Path(__file__).resolve()
    stack_root = script.parent.parent
    repo_root = stack_root.parent.parent
    binary = args.binary if args.binary.is_absolute() else (Path.cwd() / args.binary).resolve()
    fixtures = load_jsonl(args.accepted, "accepted") + load_jsonl(args.rejected, "rejected")
    identifiers = [item["fixtureId"] for item in fixtures]
    if len(set(identifiers)) != len(identifiers):
        raise FixtureError("fixtureId must be unique across accepted and rejected files")
    if not fixtures:
        raise FixtureError("fixture files contain no cases")
    validate_coverage(fixtures)
    for fixture in fixtures:
        execute_fixture(fixture, binary, repo_root, stack_root)
    print(f"PASS cli-r13 fixtures={len(fixtures)}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except FixtureError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
