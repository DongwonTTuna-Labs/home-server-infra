#!/usr/bin/env python3
"""Fail-closed verifier for the immutable PR72 R13 pre-edit baseline."""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import sys
from pathlib import Path


BASELINE_FILES = frozenset(
    {
        "HEAD.txt",
        "authority-inputs.sha256",
        "lifecycle-status.json",
        "manifest-file.sha256",
        "pr72.json",
        "remote-HEAD.txt",
        "source-manifest-identity.sha256",
        "tracked-files.sha256",
        "untracked-files.sha256",
        "upstream.txt",
        "worktree-status.sha256",
    }
)
WAIVABLE_FILES = frozenset({"wrapper-identities.sha256"})
SUPPLEMENT_NAME = "baseline-supplement.json"
SUPPLEMENT_SCHEMA = "pr72.baseline-supplement.r13.v2"
H256 = re.compile(r"^sha256:[0-9a-f]{64}$")
WRAPPER_PATHS = frozenset(
    {
        str(Path.home() / ".local/bin/gptpro"),
        str(Path.home() / ".local/bin/gptxhigh"),
        "stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle",
        "stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle-rust",
    }
)
HEX40 = re.compile(r"^[0-9a-f]{40}\n$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
SHA_LINE = re.compile(r"^(?P<digest>[0-9a-f]{64})  (?P<path>.+)$")


class BaselineError(RuntimeError):
    pass


def regular_file(path: Path) -> os.stat_result:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise BaselineError(f"required baseline file is missing: {path.name}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise BaselineError(f"baseline path is not a regular non-symlink file: {path.name}")
    return metadata


def read_text(path: Path) -> str:
    regular_file(path)
    data = path.read_bytes()
    if not data or b"\x00" in data or data.startswith(b"\xef\xbb\xbf"):
        raise BaselineError(f"baseline file has invalid bytes: {path.name}")
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise BaselineError(f"baseline file is not UTF-8: {path.name}") from error
    if "\r" in text or not text.endswith("\n"):
        raise BaselineError(f"baseline file is not LF-terminated: {path.name}")
    return text


def parse_sha_lines(path: Path, *, allow_absent: bool = False) -> dict[str, str]:
    text = read_text(path)
    if allow_absent and text == "absent\n":
        return {}
    result: dict[str, str] = {}
    for line_number, line in enumerate(text.splitlines(), 1):
        match = SHA_LINE.fullmatch(line)
        if match is None:
            raise BaselineError(f"invalid sha256sum line: {path.name}:{line_number}")
        name = match.group("path")
        if name in result:
            raise BaselineError(f"duplicate sha256sum path: {path.name}:{name}")
        result[name] = match.group("digest")
    if not result and not allow_absent:
        raise BaselineError(f"sha256sum file is empty: {path.name}")
    return result


def parse_json(path: Path) -> object:
    text = read_text(path)
    try:
        return json.loads(text, object_pairs_hook=reject_duplicate_pairs)
    except (json.JSONDecodeError, BaselineError) as error:
        raise BaselineError(f"invalid JSON baseline file: {path.name}: {error}") from error


def reject_duplicate_pairs(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise BaselineError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def nonempty_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise BaselineError(f"{field} must be non-empty text")
    return value


def parse_supplement(path: Path) -> dict[str, object]:
    value = parse_json(path)
    expected_keys = {
        "schemaVersion",
        "missingRecords",
        "reason",
        "headOidAtSupplement",
        "observedAtMs",
        "firstObservedHashes",
        "approvedAuthorityChanges",
    }
    if not isinstance(value, dict) or set(value) != expected_keys:
        raise BaselineError("baseline-supplement.json has an invalid closed field set")
    canonical = (
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n"
    ).encode()
    if path.read_bytes() != canonical:
        raise BaselineError("baseline-supplement.json is not canonical JSON")
    if value["schemaVersion"] != SUPPLEMENT_SCHEMA:
        raise BaselineError("baseline-supplement.json has the wrong schemaVersion")
    missing = value["missingRecords"]
    if (
        not isinstance(missing, list)
        or len(missing) > 16
        or any(not isinstance(item, str) or not item or "\x00" in item for item in missing)
        or len(set(missing)) != len(missing)
    ):
        raise BaselineError("baseline-supplement.json missingRecords is invalid")
    approvals = value["approvedAuthorityChanges"]
    approval_keys = {
        "path",
        "baselineSha256",
        "approvedSha256",
        "changedAtMs",
        "approvedBy",
        "reason",
    }
    if not isinstance(approvals, list):
        raise BaselineError("baseline-supplement.json approvedAuthorityChanges is invalid")
    for index, approval in enumerate(approvals):
        if not isinstance(approval, dict) or set(approval) != approval_keys:
            raise BaselineError(
                f"baseline supplement approvedAuthorityChanges[{index}] has an invalid closed field set"
            )
        nonempty_text(approval["path"], f"approvedAuthorityChanges[{index}].path")
        if (
            not isinstance(approval["baselineSha256"], str)
            or H256.fullmatch(approval["baselineSha256"]) is None
            or not isinstance(approval["approvedSha256"], str)
            or H256.fullmatch(approval["approvedSha256"]) is None
        ):
            raise BaselineError(
                f"baseline supplement approvedAuthorityChanges[{index}] has an invalid hash"
            )
        changed_at = approval["changedAtMs"]
        if (
            not isinstance(changed_at, int)
            or isinstance(changed_at, bool)
            or not 1 <= changed_at <= 9_007_199_254_740_991
        ):
            raise BaselineError(
                f"baseline supplement approvedAuthorityChanges[{index}].changedAtMs is invalid"
            )
        nonempty_text(
            approval["approvedBy"], f"approvedAuthorityChanges[{index}].approvedBy"
        )
        nonempty_text(approval["reason"], f"approvedAuthorityChanges[{index}].reason")
    if not missing and not approvals:
        raise BaselineError(
            "baseline supplement must contain a missing record or an approved authority change"
        )
    nonempty_text(value["reason"], "baseline supplement reason")
    if not isinstance(value["headOidAtSupplement"], str) or re.fullmatch(
        r"[0-9a-f]{40}", value["headOidAtSupplement"]
    ) is None:
        raise BaselineError("baseline supplement headOidAtSupplement is invalid")
    timestamp = value["observedAtMs"]
    if (
        not isinstance(timestamp, int)
        or isinstance(timestamp, bool)
        or not 1 <= timestamp <= 9_007_199_254_740_991
    ):
        raise BaselineError("baseline supplement observedAtMs is invalid")
    hashes = value["firstObservedHashes"]
    if hashes is not None:
        if (
            not isinstance(hashes, dict)
            or not hashes
            or list(hashes) != sorted(hashes)
            or any(
                not isinstance(name, str)
                or not name
                or not isinstance(digest, str)
                or H256.fullmatch(digest) is None
                for name, digest in hashes.items()
            )
        ):
            raise BaselineError("baseline supplement firstObservedHashes is invalid")
    return value


def verify_baseline(directory: Path) -> dict[str, object]:
    try:
        root_metadata = directory.lstat()
    except FileNotFoundError as error:
        raise BaselineError(f"baseline directory is missing: {directory}") from error
    if stat.S_ISLNK(root_metadata.st_mode) or not stat.S_ISDIR(root_metadata.st_mode):
        raise BaselineError("baseline root must be a non-symlink directory")

    observed = {entry.name for entry in directory.iterdir()}
    expected_records = BASELINE_FILES | WAIVABLE_FILES
    missing = sorted(expected_records - observed)
    unexpected = sorted(observed - expected_records - {SUPPLEMENT_NAME})
    if unexpected:
        raise BaselineError(f"baseline has unexpected entries: {', '.join(unexpected)}")
    supplement = (
        parse_supplement(directory / SUPPLEMENT_NAME)
        if SUPPLEMENT_NAME in observed
        else None
    )
    if missing and supplement is None:
        raise BaselineError(f"baseline is incomplete; missing: {', '.join(missing)}")
    if supplement is not None:
        if missing != supplement["missingRecords"]:
            raise BaselineError("baseline supplement missingRecords does not exactly match")
        if not set(missing).issubset(WAIVABLE_FILES):
            raise BaselineError("baseline supplement attempts to waive a non-waivable record")

    head = read_text(directory / "HEAD.txt")
    remote_head = read_text(directory / "remote-HEAD.txt")
    if HEX40.fullmatch(head) is None or HEX40.fullmatch(remote_head) is None:
        raise BaselineError("HEAD records must contain one lowercase 40-hex OID plus LF")
    upstream = read_text(directory / "upstream.txt").strip()
    if not upstream or any(character.isspace() for character in upstream):
        raise BaselineError("upstream record is invalid")

    worktree_status = read_text(directory / "worktree-status.sha256").splitlines()
    if len(worktree_status) != 1 or re.fullmatch(r"[0-9a-f]{64}  -", worktree_status[0]) is None:
        raise BaselineError("worktree-status.sha256 is invalid")

    tracked = parse_sha_lines(directory / "tracked-files.sha256")
    untracked = parse_sha_lines(directory / "untracked-files.sha256")
    overlap = sorted(set(tracked) & set(untracked))
    if overlap:
        raise BaselineError(f"tracked/untracked baseline overlap: {overlap[0]}")
    wrappers = (
        parse_sha_lines(directory / "wrapper-identities.sha256")
        if "wrapper-identities.sha256" not in missing
        else {}
    )
    authority = parse_sha_lines(directory / "authority-inputs.sha256")
    manifest = parse_sha_lines(directory / "manifest-file.sha256")
    source_manifest = parse_sha_lines(
        directory / "source-manifest-identity.sha256", allow_absent=True
    )
    if wrappers and len(wrappers) != 4:
        raise BaselineError("wrapper-identities.sha256 must contain exactly four paths")
    if len(authority) != 9:
        raise BaselineError("authority-inputs.sha256 must contain exactly nine paths")
    if len(manifest) != 1:
        raise BaselineError("manifest-file.sha256 must contain exactly one path")
    if len(source_manifest) > 1:
        raise BaselineError("source-manifest-identity.sha256 has too many paths")

    pr = parse_json(directory / "pr72.json")
    if not isinstance(pr, dict) or set(pr) != {"headRefOid", "isDraft", "mergedAt", "state"}:
        raise BaselineError("pr72.json does not have the closed Step 0 field set")
    if pr["state"] != "OPEN" or pr["isDraft"] is not True or pr["mergedAt"] is not None:
        raise BaselineError("PR #72 baseline is not open, draft, and unmerged")
    if not isinstance(pr["headRefOid"], str) or re.fullmatch(r"[0-9a-f]{40}", pr["headRefOid"]) is None:
        raise BaselineError("pr72.json headRefOid is invalid")
    lifecycle_status = parse_json(directory / "lifecycle-status.json")
    if not isinstance(lifecycle_status, dict):
        raise BaselineError("lifecycle-status.json must be a JSON object")

    if supplement is not None:
        if supplement["headOidAtSupplement"] != head.strip():
            raise BaselineError("baseline supplement HEAD differs from the baseline HEAD")
        first_observed = supplement["firstObservedHashes"]
        if missing == ["wrapper-identities.sha256"] and (
            not isinstance(first_observed, dict)
            or set(first_observed) != WRAPPER_PATHS
        ):
            raise BaselineError("wrapper waiver must record exactly four first-observed hashes")
    else:
        first_observed = None

    return {
        "authority": authority,
        "approvedAuthorityChanges": supplement["approvedAuthorityChanges"]
        if supplement is not None
        else [],
        "head": head.strip(),
        "manifest": manifest,
        "pr": pr,
        "remoteHead": remote_head.strip(),
        "sourceManifest": source_manifest,
        "tracked": tracked,
        "untracked": untracked,
        "upstream": upstream,
        "waivedRecords": missing,
        "firstObservedHashes": first_observed,
        "wrappers": wrappers,
        "worktreeStatusSha256": worktree_status[0].split()[0],
    }


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--verify", required=True, type=Path)
    args = parser.parse_args()
    try:
        summary = verify_baseline(args.verify.resolve())
    except BaselineError as error:
        print(f"baseline verification failed: {error}", file=sys.stderr)
        return 1
    print(
        "baseline verified: "
        f"tracked={len(summary['tracked'])} untracked={len(summary['untracked'])} "
        f"WAIVED={','.join(summary['waivedRecords']) if summary['waivedRecords'] else '-'}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
