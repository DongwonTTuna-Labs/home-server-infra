#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys
from datetime import datetime, timezone

STAGE = "grimoire-labels"
LABELS = {
    "casting": {"name": "🔮 Casting…", "color": "#7c3aed", "description": "Grimoire review/autofix loop is running."},
    "cast": {"name": "✨ Cast", "color": "#10b981", "description": "Grimoire review/autofix loop completed cleanly."},
    "fizzled": {"name": "💨 Fizzled", "color": "#6b7280", "description": "Grimoire review/autofix loop halted or failed closed."},
}
MANAGED = [LABELS["casting"]["name"], LABELS["cast"]["name"], LABELS["fizzled"]["name"]]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def resolve_path(raw: str, workspace: pathlib.Path) -> pathlib.Path:
    path = pathlib.Path(raw)
    if path.is_absolute():
        return path
    return workspace / path


def read_labels(path: pathlib.Path) -> list[str]:
    if not path.exists():
        return []
    labels: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        label = line.strip()
        if label and label not in labels:
            labels.append(label)
    return labels


def write_labels(path: pathlib.Path, labels: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(labels) + ("\n" if labels else ""), encoding="utf-8")


def transition_labels(current: list[str], transition: str) -> tuple[list[str], list[dict[str, str]], list[str]]:
    final = list(current)
    operations: list[dict[str, str]] = []
    notes: list[str] = []

    def remove(label: str, reason: str) -> None:
        if label in final:
            final.remove(label)
            operations.append({"action": "remove", "label": label, "reason": reason})
        else:
            notes.append(f"remove skipped for absent label: {label}")

    def add(label: str, reason: str) -> None:
        if label in final:
            notes.append(f"add skipped for existing label: {label}")
        else:
            final.append(label)
            operations.append({"action": "add", "label": label, "reason": reason})

    if transition == "running":
        if LABELS["cast"]["name"] in final or LABELS["fizzled"]["name"] in final:
            notes.append("running skipped because a terminal grimoire label is already present")
        elif LABELS["casting"]["name"] in final:
            notes.append("running skipped because Casting is already present")
        else:
            add(LABELS["casting"]["name"], "running transition adds Casting")
    elif transition == "done":
        remove(LABELS["casting"]["name"], "done transition removes running label")
        remove(LABELS["fizzled"]["name"], "done transition removes halted label")
        add(LABELS["cast"]["name"], "done transition adds Cast")
    elif transition == "fizzled":
        remove(LABELS["casting"]["name"], "fizzled transition removes running label")
        remove(LABELS["cast"]["name"], "fizzled transition removes success label")
        add(LABELS["fizzled"]["name"], "fizzled transition adds Fizzled")
    else:
        raise ValueError(f"unsupported transition: {transition}")
    return final, operations, notes


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            text = "true" if value is True else "false" if value is False else str(value)
            handle.write(f"{key}={text}\n")


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    state_file = resolve_path(args.state_file, workspace)
    state_output = resolve_path(args.state_output or args.state_file, workspace)
    status_output = resolve_path(args.status_output, workspace)
    current = read_labels(state_file)
    final, operations, notes = transition_labels(current, args.transition)
    write_labels(state_output, final)
    report = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "transition": args.transition,
        "mode": "local-state-file",
        "repo": args.repository,
        "pr_number": args.pr_number,
        "uses_default_actions_token": False,
        "labels_are_display_only": True,
        "durable_loop_state_source": False,
        "managed_labels": list(LABELS.values()),
        "unrelated_labels_preserved": sorted(label for label in final if label not in MANAGED),
        "current_labels": current,
        "final_labels": final,
        "operations": operations,
        "operation_count": len(operations),
        "changed": bool(operations),
        "github_pr_label_mutation_attempted": False,
        "notes": notes,
    }
    status_output.parent.mkdir(parents=True, exist_ok=True)
    status_output.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_github_output(args.github_output, {"changed": bool(operations), "operation_count": len(operations), "status_path": str(status_output), "state_output": str(state_output)})
    print(f"{STAGE}: transition={args.transition} operations={len(operations)} status={status_output}")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Apply idempotent display-only Grimoire label transitions in local state-file mode.")
    parser.add_argument("transition", choices=["running", "done", "fizzled"])
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--state-file", default=".omo/ci/grimoire-label-state.txt")
    parser.add_argument("--state-output", default="")
    parser.add_argument("--status-output", default=".omo/ci/grimoire-label-status.json")
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", "local-consumer"))
    parser.add_argument("--pr-number", default=os.environ.get("GRIMOIRE_PR_NUMBER", "0"))
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
