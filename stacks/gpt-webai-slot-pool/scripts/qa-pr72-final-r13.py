#!/usr/bin/env python3
"""Create fail-closed final PR72 R13 QA evidence from an immutable baseline."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path

MANIFEST_SHA256 = "aac90df1f946bc74443174c3c2c02d7ea0bea15ac069856eea4c65e2b544ec38"
R23_NAME = "IMPLEMENTATION_DECISIONS_R23.md"
R24_NAME = "IMPLEMENTATION_DECISIONS_R24.md"
R25_NAME = "IMPLEMENTATION_DECISIONS_R25.md"
R26_NAME = "IMPLEMENTATION_DECISIONS_R26.md"
R27_NAME = "IMPLEMENTATION_DECISIONS_R27.md"
R27_SHA256 = "f1796914d750989765a149965a497f7877c2ed1077e7e2f71e86b508a84aa9d5"
R28_NAME = "IMPLEMENTATION_DECISIONS_R28.md"
R28_SHA256 = "a9cb73b88d16af9e346228bef83d820ec092d2f082d3dacd2731108eb76d3e4e"
R24_DISPOSITION_SUPPLEMENT = {
    "stacks/gpt-webai-slot-pool/scripts/slot-entrypoint.sh": "modify",
    "stacks/gpt-webai-slot-pool/tests/gpt-webai-lifecycle/fixtures/fake-bin/agbrowse": "delete",
}
R25_DISPOSITION_SUPPLEMENT = {
    ".omo/evidence/gpt-webai-lifecycle/pre-edit-r13/baseline-supplement.json": "new",
    "stacks/gpt-webai-slot-pool/compose.fake.yaml": "new",
    "stacks/gpt-webai-slot-pool/scripts/qa-live-matrix-cases.r13.tsv": "new",
}
CONDITIONAL_MODIFY_SUPPLEMENT = {
    "stacks/gpt-webai-slot-pool/Cargo.lock": "modify",
    "stacks/gpt-webai-slot-pool/Dockerfile": "modify",
    "stacks/gpt-webai-slot-pool/compose.yaml": "modify",
    "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/Cargo.toml": "modify",
}
R26_DISPOSITION_SUPPLEMENT = {
    "removals": frozenset(
        {
            "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/artifacts/",
            "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/events/",
            "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/projections/",
            "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/receipts/",
            "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/recovery/",
        }
    ),
    "additions": {
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/contracts/health.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/failpoint.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_assets.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_browser.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_events/binding.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_events/bootstrap.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_events/mod.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_events/send.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_events/upload.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_provider.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_send_flow.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_types.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/request/r13_upload.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/runtime/docker_control.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/artifacts.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/executor.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/journal.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/provider.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/release.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/release/partial.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/runtime_r13.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/src/session_ops/terminal.rs": "new",
        "stacks/gpt-webai-slot-pool/crates/gpt-webai-lifecycle/tests/cli_status.rs": "new",
        "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/": "generated",
    },
}
R26_DISPOSITION_NOTES = {
    path: "R26-D6b" for path in R26_DISPOSITION_SUPPLEMENT["additions"]
}
R26_DISPOSITION_NOTES[
    "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/"
] = "R26-D6a"
INTENDED_NONSTACK_BASELINE_CHANGES = frozenset({".fable-sol/state.md"})


class FinalQaError(RuntimeError):
    pass


def load_baseline_module() -> object:
    source = Path(__file__).with_name("qa-pr72-baseline-r13.py")
    spec = importlib.util.spec_from_file_location("qa_pr72_baseline_r13", source)
    if spec is None or spec.loader is None:
        raise FinalQaError("cannot load baseline verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(argv: list[str], cwd: Path, *, allow_failure: bool = False) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if completed.returncode != 0 and not allow_failure:
        stderr = completed.stderr.decode("utf-8", "replace").strip()
        raise FinalQaError(f"command failed ({completed.returncode}): {' '.join(argv)}: {stderr}")
    return completed


def sha256(path: Path) -> str:
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise FinalQaError(f"not a regular non-symlink file: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repo_root_from_script() -> Path:
    return Path(__file__).resolve().parents[3]


def verify_canonical(repo_root: Path) -> dict[str, str]:
    canonical = repo_root / ".omo/plans/pr72-canonical-design"
    manifest = canonical / "MANIFEST.sha256"
    if sha256(manifest) != MANIFEST_SHA256:
        raise FinalQaError("canonical manifest file identity changed")
    check = run(["sha256sum", "-c", "MANIFEST.sha256"], canonical)
    addendum = canonical / R23_NAME
    r24 = canonical / R24_NAME
    r25 = canonical / R25_NAME
    r26 = canonical / R26_NAME
    r27 = canonical / R27_NAME
    r28 = canonical / R28_NAME
    if not addendum.is_file() or addendum.is_symlink():
        raise FinalQaError("R23 addendum is missing or not a regular file")
    if not r24.is_file() or r24.is_symlink():
        raise FinalQaError("R24 addendum is missing or not a regular file")
    if not r25.is_file() or r25.is_symlink():
        raise FinalQaError("R25 addendum is missing or not a regular file")
    if not r26.is_file() or r26.is_symlink():
        raise FinalQaError("R26 addendum is missing or not a regular file")
    if not r27.is_file() or r27.is_symlink() or sha256(r27) != R27_SHA256:
        raise FinalQaError("R27 addendum identity changed or is missing")
    if not r28.is_file() or r28.is_symlink() or sha256(r28) != R28_SHA256:
        raise FinalQaError("R28 addendum identity changed or is missing")
    return {
        "manifestSha256": MANIFEST_SHA256,
        "r23Sha256": sha256(addendum),
        "r24Sha256": sha256(r24),
        "r25Sha256": sha256(r25),
        "r26Sha256": sha256(r26),
        "r27Sha256": R27_SHA256,
        "r28Sha256": R28_SHA256,
        "checkStdoutSha256": hashlib.sha256(check.stdout).hexdigest(),
    }


def verify_git_and_pr(repo_root: Path, baseline: dict[str, object]) -> dict[str, object]:
    branch = run(["git", "branch", "--show-current"], repo_root).stdout.decode().strip()
    if branch != "codex/gpt-webai-slot-pool":
        raise FinalQaError(f"unexpected branch: {branch}")
    head = run(["git", "rev-parse", "HEAD"], repo_root).stdout.decode().strip()
    if head != baseline["head"]:
        raise FinalQaError("HEAD changed before the approved intended commit")
    upstream = run(
        ["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], repo_root
    ).stdout.decode().strip()
    if upstream != baseline["upstream"]:
        raise FinalQaError("upstream branch changed")
    pr_output = run(
        ["gh", "pr", "view", "72", "--json", "state,isDraft,headRefOid,mergedAt"], repo_root
    ).stdout
    try:
        pr = json.loads(pr_output)
    except json.JSONDecodeError as error:
        raise FinalQaError("current PR JSON is invalid") from error
    if set(pr) != {"state", "isDraft", "headRefOid", "mergedAt"}:
        raise FinalQaError("current PR JSON has unexpected fields")
    if pr["state"] != "OPEN" or pr["isDraft"] is not True or pr["mergedAt"] is not None:
        raise FinalQaError("PR #72 is not open, draft, and unmerged")
    return {"branch": branch, "head": head, "pr": pr, "upstream": upstream}


def baseline_path(repo_root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else repo_root / path


def verify_baseline_identity_map(
    repo_root: Path,
    identities: object,
    *,
    label: str,
    allowed_changes: frozenset[str] = frozenset(),
    approved_changes: object = (),
    resolved_changes: list[dict[str, object]] | None = None,
) -> dict[str, str]:
    if not isinstance(identities, dict) or not identities:
        raise FinalQaError(f"baseline {label} identities are missing")
    observed: dict[str, str] = {}
    for value, expected in identities.items():
        if not isinstance(value, str) or not isinstance(expected, str):
            raise FinalQaError(f"baseline {label} identity is invalid")
        path = baseline_path(repo_root, value)
        current = sha256(path)
        observed[value] = current
        if value in allowed_changes or current == expected:
            continue
        expected_baseline = f"sha256:{expected}"
        expected_approved = f"sha256:{current}"
        if not isinstance(approved_changes, (list, tuple)):
            raise FinalQaError(f"baseline {label} approved changes are invalid")
        matches = [
            entry
            for entry in approved_changes
            if isinstance(entry, dict)
            and entry.get("path") == value
            and entry.get("baselineSha256") == expected_baseline
            and entry.get("approvedSha256") == expected_approved
        ]
        if not matches:
            raise FinalQaError(f"baseline {label} identity changed: {value}")
        if resolved_changes is not None:
            resolved_changes.extend(dict(entry) for entry in matches)
    return observed


def verify_baseline_identities(
    repo_root: Path, baseline: dict[str, object]
) -> tuple[dict[str, dict[str, str]], list[dict[str, object]]]:
    wrappers_source = baseline.get("wrappers")
    wrappers = (
        verify_baseline_identity_map(repo_root, wrappers_source, label="wrapper")
        if isinstance(wrappers_source, dict) and wrappers_source
        else {}
    )
    first_observed_source = baseline.get("firstObservedHashes")
    first_observed = {}
    if first_observed_source is not None:
        if not isinstance(first_observed_source, dict) or not first_observed_source:
            raise FinalQaError("baseline first-observed identities are invalid")
        normalized = {
            path: digest.removeprefix("sha256:")
            for path, digest in first_observed_source.items()
        }
        first_observed = verify_baseline_identity_map(
            repo_root, normalized, label="first-observed wrapper"
        )
    # These three repository documents are intentionally `modify` paths. Their
    # preservation is governed by the disposition-aware tracked-file check;
    # every other authority input is external/keep and must retain its Step 0
    # identity exactly.
    modified_authority = frozenset(
        {
            "stacks/gpt-webai-slot-pool/README.md",
            "stacks/gpt-webai-slot-pool/SMOKE_TESTS.md",
            "stacks/gpt-webai-slot-pool/docs/gpt-webai-lifecycle-runbook.md",
        }
    )
    approved_authority_changes: list[dict[str, object]] = []
    authority = verify_baseline_identity_map(
        repo_root,
        baseline.get("authority"),
        label="authority",
        allowed_changes=modified_authority,
        approved_changes=baseline.get("approvedAuthorityChanges", []),
        resolved_changes=approved_authority_changes,
    )
    manifest = verify_baseline_identity_map(
        repo_root,
        baseline.get("manifest"),
        label="canonical manifest",
        approved_changes=baseline.get("approvedAuthorityChanges", []),
        resolved_changes=approved_authority_changes,
    )
    source_manifest_identities = baseline.get("sourceManifest")
    if not isinstance(source_manifest_identities, dict):
        raise FinalQaError("baseline source-manifest identities are invalid")
    source_manifest = (
        verify_baseline_identity_map(
            repo_root,
            source_manifest_identities,
            label="source manifest",
        )
        if source_manifest_identities
        else {}
    )
    return (
        {
            "authority": authority,
            "manifest": manifest,
            "sourceManifest": source_manifest,
            "firstObserved": first_observed,
            "wrappers": wrappers,
        },
        approved_authority_changes,
    )


def parse_dispositions(path: Path) -> dict[str, str]:
    data = path.read_bytes()
    if data.startswith(b"\xef\xbb\xbf") or b"\r" in data or not data.endswith(b"\n"):
        raise FinalQaError("qa-file-disposition.r13.tsv serialization is invalid")
    lines = data.decode("utf-8").splitlines()
    if not lines or lines[0] != "path\tdisposition\tnote":
        raise FinalQaError("qa-file-disposition.r13.tsv header is invalid")
    result: dict[str, str] = {}
    notes: dict[str, str] = {}
    previous: str | None = None
    for line_number, line in enumerate(lines[1:], 2):
        fields = line.split("\t")
        if len(fields) != 3 or fields[1] not in {
            "keep", "modify", "new", "generated", "external_keep", "delete"
        }:
            raise FinalQaError(f"invalid disposition row {line_number}")
        item, disposition, note = fields
        if not item or not note or item in result or (previous is not None and item <= previous):
            raise FinalQaError(f"unsorted/duplicate disposition row {line_number}")
        result[item] = disposition
        notes[item] = note
        previous = item
    if not result:
        raise FinalQaError("qa-file-disposition.r13.tsv is empty")
    for item, expected_note in R26_DISPOSITION_NOTES.items():
        if notes.get(item) != expected_note:
            raise FinalQaError(f"R26 disposition note differs for {item}")
    return result


def expand_braces(value: str) -> list[str]:
    start = value.find("{")
    if start < 0:
        return [value]
    end = value.find("}", start)
    if end < 0:
        raise FinalQaError(f"unclosed disposition brace expression: {value}")
    expanded: list[str] = []
    for member in value[start + 1 : end].split(","):
        expanded.extend(expand_braces(value[:start] + member + value[end + 1 :]))
    return expanded


def module_code_lines(document: str, heading: str, next_heading: str) -> list[str]:
    try:
        section = document.split(heading, 1)[1].split(next_heading, 1)[0]
    except IndexError as error:
        raise FinalQaError(f"missing MODULE disposition heading: {heading}") from error
    blocks = re.findall(r"```text\n(.*?)\n```", section, flags=re.DOTALL)
    if len(blocks) != 1:
        raise FinalQaError(f"unexpected MODULE code-block count for {heading}: {len(blocks)}")
    return [line for line in blocks[0].splitlines() if line]


def expected_dispositions(repo_root: Path) -> dict[str, str]:
    module_path = repo_root / ".omo/plans/pr72-canonical-design/MODULE_AND_FILE_PLAN.md"
    document = module_path.read_text(encoding="utf-8")
    try:
        section_six = document.split(
            "## 6. Current source manifest disposition, all 243 files", 1
        )[1].split("## 7.", 1)[0]
    except IndexError as error:
        raise FinalQaError("MODULE section 6 disposition inventory is missing") from error
    expected: dict[str, str] = {}
    row_pattern = re.compile(r"\| `([^`]+)` \| `[0-9a-f]{64}` \| ([^|]+?) \|")
    for match in row_pattern.finditer(section_six):
        source_path, prose = match.groups()
        path = source_path.removeprefix("source/")
        disposition = prose.strip().split()[0].strip("`")
        if disposition not in {"keep", "modify"} or path in expected:
            raise FinalQaError(f"invalid MODULE section 6 disposition row: {path}")
        expected[path] = disposition
    if len(expected) != 243:
        raise FinalQaError(f"MODULE section 6 must contain 243 rows, found {len(expected)}")

    stack_prefix = "stacks/gpt-webai-slot-pool/"
    generated = module_code_lines(
        document,
        "### Generated/fixture/QA data",
        "### R13 test and QA acceptance paths",
    )
    new = module_code_lines(document, "### Rust new modules", "### Node new modules")
    new += module_code_lines(
        document,
        "### Node new modules",
        "### Generated/fixture/QA data",
    )
    new += module_code_lines(
        document,
        "### R13 test and QA acceptance paths (all `new`)",
        "Each Rust integration test target",
    )
    for value in generated:
        for path in expand_braces(value):
            expected[stack_prefix + path] = "generated"
    expected[
        stack_prefix + "contracts/ui-labels-r14/chip-removal-labels.tsv"
    ] = "generated"
    for value in new:
        for path in expand_braces(value):
            expected[stack_prefix + path] = "new"
    expected.update(R24_DISPOSITION_SUPPLEMENT)
    expected.update(R25_DISPOSITION_SUPPLEMENT)
    expected.update(CONDITIONAL_MODIFY_SUPPLEMENT)
    for removed in R26_DISPOSITION_SUPPLEMENT["removals"]:
        if expected.pop(removed, None) != "generated":
            raise FinalQaError(f"R26 disposition removal is not a generated MODULE path: {removed}")
    expected.update(R26_DISPOSITION_SUPPLEMENT["additions"])
    rust_test_prefix = stack_prefix + "crates/gpt-webai-lifecycle/tests/"
    test_targets = [
        path
        for path, disposition in expected.items()
        if disposition == "new"
        and path.startswith(rust_test_prefix)
        and path.endswith(".rs")
        and "/" not in path[len(rust_test_prefix) : -3]
    ]
    for target in test_targets:
        helper_root = repo_root / target[:-3]
        if not helper_root.is_dir() or helper_root.is_symlink():
            continue
        for helper in sorted(helper_root.rglob("*.rs")):
            if helper.is_symlink() or not helper.is_file():
                raise FinalQaError(f"invalid Rust test helper path: {helper}")
            expected[helper.relative_to(repo_root).as_posix()] = "new"
    return expected


def verify_disposition_coverage(repo_root: Path, dispositions: dict[str, str]) -> None:
    expected = expected_dispositions(repo_root)
    missing = sorted(set(expected) - set(dispositions))
    mismatched = sorted(
        path
        for path in set(expected) & set(dispositions)
        if expected[path] != dispositions[path]
    )
    if missing or mismatched:
        raise FinalQaError(
            "qa disposition coverage differs from MODULE section 6/4: "
            f"missing={missing[:5]} mismatched={mismatched[:5]}"
        )

    tracked = {
        raw.decode("utf-8", "strict")
        for raw in run(
            ["git", "ls-files", "-z", "--", "stacks/gpt-webai-slot-pool"],
            repo_root,
        ).stdout.split(b"\0")
        if raw
    }
    extra = sorted(set(dispositions) - set(expected))
    if extra:
        raise FinalQaError(
            "qa disposition has paths outside MODULE section 6/4 and the R24/R25/R26 supplements: "
            f"{extra[:5]}"
        )
    for path in tracked:
        if disposition_for_path(path, dispositions) is None:
            raise FinalQaError(f"tracked stack path has no disposition: {path}")

    for relative, disposition in dispositions.items():
        path = repo_root / relative.rstrip("/")
        present = path.exists() or path.is_symlink()
        if disposition == "delete" and present:
            raise FinalQaError(f"delete disposition path still exists: {relative}")
        if disposition != "delete" and not present:
            raise FinalQaError(f"disposition path is absent: {relative}")


def verify_unrelated(repo_root: Path, baseline: dict[str, object], dispositions: dict[str, str]) -> None:
    mutable_prefix = "stacks/gpt-webai-slot-pool/"
    tracked = baseline["tracked"]
    untracked = baseline["untracked"]
    assert isinstance(tracked, dict)
    assert isinstance(untracked, dict)
    approved_changes = baseline.get("approvedAuthorityChanges", [])
    if not isinstance(approved_changes, (list, tuple)):
        raise FinalQaError("baseline approvedAuthorityChanges are invalid")

    def verify_paths(records: dict[str, str]) -> None:
        for relative, expected in records.items():
            if relative in INTENDED_NONSTACK_BASELINE_CHANGES:
                continue
            if relative.startswith(mutable_prefix):
                disposition = disposition_for_path(relative, dispositions)
                if disposition is None:
                    raise FinalQaError(f"baseline stack path has no disposition: {relative}")
                if disposition in {"modify", "new", "generated", "delete"}:
                    continue
            path = repo_root / relative
            if not path.exists() or path.is_symlink() or not path.is_file():
                raise FinalQaError(f"unrelated/keep baseline path is absent or unsafe: {relative}")
            current = sha256(path)
            if current == expected:
                continue
            # A keep-path change is admissible only through the same fail-closed,
            # exact-triple-matched approval channel that governs authority/manifest identity.
            expected_baseline = f"sha256:{expected}"
            expected_approved = f"sha256:{current}"
            approved = any(
                isinstance(entry, dict)
                and entry.get("path") == relative
                and entry.get("baselineSha256") == expected_baseline
                and entry.get("approvedSha256") == expected_approved
                for entry in approved_changes
            )
            if not approved:
                raise FinalQaError(f"unrelated/keep baseline path changed: {relative}")

    verify_paths(tracked)
    verify_paths(untracked)


def disposition_for_path(relative: str, dispositions: dict[str, str]) -> str | None:
    disposition = dispositions.get(relative)
    if disposition is not None:
        return disposition
    fixture_root = "stacks/gpt-webai-slot-pool/tests/fixtures/lifecycle-r13/"
    if relative.startswith(fixture_root):
        return dispositions.get(fixture_root)
    return None


def source_fingerprint(repo_root: Path) -> str:
    script = repo_root / "stacks/gpt-webai-slot-pool/scripts/qa-fingerprint-r13.sh"
    completed = run(["bash", str(script), "--print"], repo_root)
    value = completed.stdout.decode("ascii", "strict")
    if re.fullmatch(r"[0-9a-f]{64}\n", value) is None:
        raise FinalQaError("source fingerprint output is not one lowercase hex line")
    return value.strip()


def create_output(path: Path, record: dict[str, object]) -> None:
    if path.exists() or path.is_symlink():
        raise FinalQaError(f"final evidence output already exists: {path}")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.mkdir(mode=0o700)
    payload = (json.dumps(record, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    target = path / "final-summary.json"
    descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        directory = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except Exception:
        try:
            target.unlink()
            path.rmdir()
        except OSError:
            pass
        raise


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    repo_root = repo_root_from_script()
    try:
        baseline_module = load_baseline_module()
        baseline = baseline_module.verify_baseline(args.baseline.resolve())
        canonical = verify_canonical(repo_root)
        disposition_path = repo_root / "stacks/gpt-webai-slot-pool/scripts/qa-file-disposition.r13.tsv"
        dispositions = parse_dispositions(disposition_path)
        verify_disposition_coverage(repo_root, dispositions)
        verify_unrelated(repo_root, baseline, dispositions)
        baseline_identities, approved_authority_changes = verify_baseline_identities(
            repo_root, baseline
        )
        git_pr = verify_git_and_pr(repo_root, baseline)
        fingerprint = source_fingerprint(repo_root)
        record = {
            "baselineIdentities": baseline_identities,
            "approvedAuthorityChanges": approved_authority_changes,
            "baselineHead": baseline["head"],
            "canonical": canonical,
            "git": git_pr,
            "schemaVersion": "pr72.final-qa.r13.v1",
            "sourceFingerprint": fingerprint,
            "waivedRecords": baseline.get("waivedRecords", []),
            "residualRisks": [
                "wrapper-identities.sha256 has no pre-edit bytes; current hashes are first-observed only"
            ]
            if baseline.get("waivedRecords")
            else [],
        }
        create_output(args.output.resolve(), record)
    except (FinalQaError, OSError, RuntimeError, UnicodeError) as error:
        print(f"final QA failed: {error}", file=sys.stderr)
        return 1
    print(f"final QA evidence created: {args.output.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
