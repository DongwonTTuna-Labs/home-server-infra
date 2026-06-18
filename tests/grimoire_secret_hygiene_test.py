#!/usr/bin/env python3
# pyright: reportAny=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import pathlib
import re
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
APP_TOKEN_EXPR = "${{ steps.grimoire-app-token.outputs.token }}"
APP_TOKEN_ACTION = "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349"
APP_AUTH_FILES = (
    ".github/workflows/grimoire-control-plane.yml",
    "actions/grimoire/labels/action.yml",
    "actions/grimoire/labels/scripts/labels.py",
    "actions/grimoire/cast/action.yml",
    "actions/grimoire/cast/scripts/cast_driver.py",
)
SENTINELS = (
    "GRIMOIRE_FIXTURE_CODEX_RELAY_API_KEY_SENTINEL",
    "GRIMOIRE_FIXTURE_CODEX_LOOP_PAT_SENTINEL",
    "GRIMOIRE_FIXTURE_AI_RELAY_API_KEY_SENTINEL",
    "GRIMOIRE_FIXTURE_CF_ACCESS_CLIENT_ID_SENTINEL",
    "GRIMOIRE_FIXTURE_CF_ACCESS_CLIENT_SECRET_SENTINEL",
)
GITHUB_PAT_PREFIX = "github" + "_pat_"
OPENAI_TOKEN_PREFIX = "sk" + "-"
TOKEN_PATTERNS = (
    re.compile(rf"(?<![A-Za-z0-9_]){GITHUB_PAT_PREFIX}[A-Za-z0-9_]{{20,}}"),
    re.compile(r"(?<![A-Za-z0-9_])gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(rf"(?<![A-Za-z0-9_-]){OPENAI_TOKEN_PREFIX}[A-Za-z0-9_-]{{20,}}"),
    re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._~-]{16,}"),
    re.compile(r"(?i)https?://[^\s\"'<>]+(?:token|access_token|api[_-]?key)=[^\s\"'<>]+"),
    re.compile(r"(?i)\b(?:CODEX_RELAY_API_KEY|CODEX_LOOP_PAT|AI_RELAY_API_KEY|GRIMOIRE_PAT|CF_ACCESS_CLIENT_ID|CF_ACCESS_CLIENT_SECRET)\s*[:=]\s*[\"']?[^\"'\s]+"),
    re.compile(r"(?i)\b(?:token|secret|password|api[_-]?key)\s*[:=]\s*[\"']?[^\"'\s]+"),
)
RAW_SECRET_MATERIAL_PATTERNS = (
    ("private-key", re.compile(r"-{5}BEGIN [A-Z0-9 ]*PRIVATE KEY-{5}")),
    ("token-pattern", TOKEN_PATTERNS[0]),
    ("token-pattern", TOKEN_PATTERNS[1]),
    ("token-pattern", TOKEN_PATTERNS[2]),
    ("token-pattern", TOKEN_PATTERNS[3]),
    ("token-bearing-url", TOKEN_PATTERNS[4]),
    ("private-run-url", re.compile(r"https://github\.com/DongwonTTuna-Labs/[^/\s\"'<>]+/actions/runs/\d+")),
)
LEGACY_LIVE_AUTH_PATTERNS = (
    re.compile(r"(?i)\bPAT-only\b"),
    re.compile(r"(?i)\bGRIMOIRE_PAT\b.*\b(?:preferred|privileged|auth|secret|fallback|map|using|uses)\b"),
    re.compile(r"(?i)\bCODEX_LOOP_PAT\b.*\b(?:preferred|privileged|auth|secret|fallback|available|using|uses)\b"),
)
SAFE_LEGACY_CONTEXT_MARKERS = (
    "Invalid example",
    "do not copy",
    "forbidden",
    "Don't use",
    "must not",
    "no legacy",
    "replaced",
)


class HygieneError(AssertionError):
    pass


def repo_text(relative: str) -> str:
    return (REPO_ROOT / relative).read_text(encoding="utf-8", errors="replace")


def scan_targets(logs: list[pathlib.Path], artifact_roots: list[pathlib.Path]) -> list[pathlib.Path]:
    targets: list[pathlib.Path] = []
    seen: set[pathlib.Path] = set()
    for raw in [*logs, *artifact_roots]:
        if not raw.exists():
            raise HygieneError(f"scan target does not exist: {raw}")
    for log in logs:
        path = log.resolve()
        if path not in seen:
            targets.append(path)
            seen.add(path)
    for root in artifact_roots:
        resolved_root = root.resolve()
        for path in sorted(resolved_root.rglob("*")):
            if path.is_symlink() or not path.is_file():
                continue
            resolved = path.resolve()
            if resolved not in seen:
                targets.append(resolved)
                seen.add(resolved)
    return targets


def classes_for(text: str) -> set[str]:
    findings: set[str] = set()
    if any(sentinel in text for sentinel in SENTINELS):
        findings.add("sentinel")
    if any(pattern.search(text) for pattern in TOKEN_PATTERNS):
        findings.add("token-pattern")
    if RAW_SECRET_MATERIAL_PATTERNS[0][1].search(text):
        findings.add("private-key")
    return findings


def raw_material_classes_for(text: str) -> set[str]:
    return {class_name for class_name, pattern in RAW_SECRET_MATERIAL_PATTERNS if pattern.search(text)}


def scan_file(path: pathlib.Path) -> set[str]:
    text = path.read_text(encoding="utf-8", errors="replace")
    return classes_for(text)


def scan(logs: list[pathlib.Path], artifact_roots: list[pathlib.Path]) -> list[tuple[pathlib.Path, str]]:
    findings: list[tuple[pathlib.Path, str]] = []
    for path in scan_targets(logs, artifact_roots):
        for class_name in sorted(scan_file(path)):
            findings.append((path, class_name))
    return findings


def iter_text_files(roots: tuple[pathlib.Path, ...]) -> list[pathlib.Path]:
    paths: list[pathlib.Path] = []
    for root in roots:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*")):
            if path.is_symlink() or not path.is_file():
                continue
            if "__pycache__" in path.parts or path.suffix in {".pyc", ".png", ".jpg", ".jpeg", ".gif"}:
                continue
            paths.append(path)
    return paths


def has_safe_legacy_context(lines: list[str], index: int) -> bool:
    start = max(0, index - 4)
    context = "\n".join(lines[start : index + 1])
    return any(marker in context for marker in SAFE_LEGACY_CONTEXT_MARKERS)


def legacy_live_auth_findings(text: str) -> list[tuple[int, str]]:
    findings: list[tuple[int, str]] = []
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if has_safe_legacy_context(lines, index):
            continue
        if any(pattern.search(line) for pattern in LEGACY_LIVE_AUTH_PATTERNS):
            findings.append((index + 1, line.strip()))
    return findings


def test_app_private_key_names_are_allowed_but_secret_material_is_rejected() -> None:
    allowed_names = "GRIMOIRE_APP_PRIVATE_KEY grimoire_app_client_id app_id client_id"
    assert classes_for(allowed_names) == set()
    assert raw_material_classes_for(allowed_names) == set()

    private_key_block = "\n".join(("-----BEGIN " + "PRIVATE KEY-----", "fixture", "-----END " + "PRIVATE KEY-----"))
    github_token = GITHUB_PAT_PREFIX + ("A" * 24)
    openai_token = OPENAI_TOKEN_PREFIX + ("A" * 24)
    legacy_name = "GRIMOIRE" + "_PAT"
    legacy_mapping = legacy_name + ": ${{ secrets." + legacy_name + " }}"

    assert "private-key" in classes_for(private_key_block)
    assert "token-pattern" in classes_for(github_token)
    assert "token-pattern" in classes_for(openai_token)
    assert "token-pattern" in classes_for(legacy_mapping)
    assert "token-pattern" in raw_material_classes_for(github_token)
    assert "token-pattern" in raw_material_classes_for(openai_token)


def test_reusable_workflow_privileged_auth_comes_only_from_minted_app_token() -> None:
    workflow = repo_text(".github/workflows/grimoire-control-plane.yml")
    forbidden_sources = ("GRIMOIRE_PAT", "CODEX_LOOP_PAT", "GITHUB_TOKEN", "github.token", "steps.auth.outputs.github_pat")

    assert workflow.count("actions/create-github-app-token@") == 1
    assert APP_TOKEN_ACTION in workflow
    assert "id: grimoire-app-token" in workflow
    assert "client-id: ${{ inputs.grimoire_app_client_id }}" in workflow
    assert "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}" in workflow
    assert "owner: DongwonTTuna-Labs" in workflow
    assert workflow.count(f"token: {APP_TOKEN_EXPR}") == 2
    assert f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}" in workflow
    for source in forbidden_sources:
        assert source not in workflow


def test_labels_and_cast_consumers_have_no_live_pat_fallback() -> None:
    labels_action = repo_text("actions/grimoire/labels/action.yml")
    labels_script = repo_text("actions/grimoire/labels/scripts/labels.py")
    cast_action = repo_text("actions/grimoire/cast/action.yml")
    cast_script = repo_text("actions/grimoire/cast/scripts/cast_driver.py")
    live_auth_text = "\n".join((labels_action, labels_script, cast_action, cast_script))

    assert "GRIMOIRE_LABEL_TOKEN: ${{ inputs.token }}" in labels_action
    assert 'os.environ.get("GRIMOIRE_LABEL_TOKEN", os.environ.get("GRIMOIRE_GITHUB_PAT", ""))' in labels_script
    assert cast_script.count('os.environ.get("GRIMOIRE_GITHUB_PAT", "")') >= 3
    for step_id in ("labels-running", "labels-done", "labels-fizzled", "labels-spec-needed"):
        marker = f"- id: {step_id}"
        start = cast_action.find(marker)
        assert start != -1, f"missing {step_id}"
        next_step = cast_action.find("\n    - id:", start + len(marker))
        block = cast_action[start:] if next_step == -1 else cast_action[start:next_step]
        assert "remote-apply: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT != '' }}" in block
        assert "token: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT || '' }}" in block

    forbidden_live_phrases = ("Resolved Grimoire PAT", "resolved PAT auth", "pat-input", "PAT-only", "CODEX_LOOP_PAT fallback")
    forbidden_runtime_gets = ('os.environ.get("GRIMOIRE_PAT"', 'os.environ.get("CODEX_LOOP_PAT"', "${{ secrets.GRIMOIRE_PAT }}", "${{ secrets.CODEX_LOOP_PAT }}", "${{ secrets.GITHUB_TOKEN }}", "${{ github.token }}")
    for phrase in forbidden_live_phrases + forbidden_runtime_gets:
        assert phrase not in live_auth_text


def test_current_consumer_docs_do_not_document_live_legacy_pat_auth() -> None:
    docs_text = repo_text("docs/grimoire-reusable.md")
    assert legacy_live_auth_findings(docs_text) == []


def test_docs_fixtures_and_evidence_do_not_expose_raw_secret_material() -> None:
    roots = (REPO_ROOT / "docs", REPO_ROOT / ".omo" / "evidence", REPO_ROOT / "tests" / "fixtures")
    findings: list[str] = []
    for path in iter_text_files(roots):
        classes = raw_material_classes_for(path.read_text(encoding="utf-8", errors="replace"))
        for class_name in sorted(classes):
            findings.append(f"{path.relative_to(REPO_ROOT)}: {class_name}")
    assert findings == []


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Fail if Grimoire fixture logs or artifacts expose secret-shaped material.")
    parser.add_argument("--log", action="append", type=pathlib.Path, default=[])
    parser.add_argument("--artifact-root", action="append", type=pathlib.Path, default=[])
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if not args.log and not args.artifact_root:
        print("at least one --log or --artifact-root is required", file=sys.stderr)
        return 2
    try:
        findings = scan(args.log, args.artifact_root)
    except (HygieneError, OSError) as exc:
        print(str(exc), file=sys.stderr)
        return 2
    if findings:
        for path, class_name in findings:
            print(f"{path}: {class_name}", file=sys.stderr)
        return 1
    print("secret hygiene ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
