#!/usr/bin/env python3
# pyright: reportAny=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import pathlib
import re
import sys

SENTINELS = (
    "GRIMOIRE_FIXTURE_CODEX_RELAY_API_KEY_SENTINEL",
    "GRIMOIRE_FIXTURE_CODEX_LOOP_PAT_SENTINEL",
    "GRIMOIRE_FIXTURE_AI_RELAY_API_KEY_SENTINEL",
)
TOKEN_PATTERNS = (
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._~-]{16,}"),
    re.compile(r"(?i)https?://[^\s\"'<>]+(?:token|access_token|api[_-]?key)=[^\s\"'<>]+"),
    re.compile(r"(?i)\b(?:CODEX_RELAY_API_KEY|CODEX_LOOP_PAT|AI_RELAY_API_KEY|GRIMOIRE_PAT)\s*[:=]\s*[\"']?[^\"'\s]+"),
    re.compile(r"(?i)\b(?:token|secret|password|api[_-]?key)\s*[:=]\s*[\"']?[^\"'\s]+"),
)


class HygieneError(AssertionError):
    pass


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
    return findings


def scan_file(path: pathlib.Path) -> set[str]:
    text = path.read_text(encoding="utf-8", errors="replace")
    return classes_for(text)


def scan(logs: list[pathlib.Path], artifact_roots: list[pathlib.Path]) -> list[tuple[pathlib.Path, str]]:
    findings: list[tuple[pathlib.Path, str]] = []
    for path in scan_targets(logs, artifact_roots):
        for class_name in sorted(scan_file(path)):
            findings.append((path, class_name))
    return findings


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
