#!/usr/bin/env bash
set -euo pipefail

python3 - <<'PY'
import base64
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ORG = os.environ.get("RUNNER_ORG", "DongwonTTuna-Labs")
RUNNER_GROUP_ID = int(os.environ.get("RUNNER_GROUP_ID", "3"))
RUNNER_GROUP_SCOPE = os.environ.get("RUNNER_GROUP_SCOPE", "all").strip().lower()
OLD_OWNER = os.environ.get("OLD_OWNER", "DongwonTTuna")
REPOS = [
    "rs-builder-relayer-client",
    "polymarket-liquidity-farming-rs",
    "bioden",
]
WORKFLOW_PATHS = [
    ".github/workflows/codex-pr-review-pipeline.yml",
    ".github/workflows/codex-pr-review-on-comment.yml",
]
MESSAGE = "Route Codex workflows to org runner pool"

token = os.environ.get("GITHUB_PAT", "").strip()
if not token:
    token_file = Path("state/github_pat")
    if token_file.exists():
        token = token_file.read_text(encoding="utf-8").strip()

if not token:
    raise SystemExit("GITHUB_PAT or state/github_pat is required")

headers = {
    "Authorization": f"Bearer {token}",
    "Accept": "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "codex-org-pool-migration",
}


def request(method, path, payload=None, ok=(200, 201, 202, 204)):
    url = f"https://api.github.com{path}"
    data = None
    req_headers = dict(headers)
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        req_headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=req_headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            body = resp.read().decode("utf-8", "replace")
            parsed = json.loads(body) if body else None
            if resp.status not in ok:
                raise RuntimeError(f"{method} {path} returned {resp.status}: {body[:500]}")
            return resp.status, parsed
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path} failed {exc.code}: {body[:800]}") from exc


def repo_exists(owner, repo):
    try:
        _, data = request("GET", f"/repos/{owner}/{repo}", ok=(200,))
        return data
    except RuntimeError:
        return None


def transfer_repo(repo):
    if repo_exists(ORG, repo):
        print(f"{repo}: already owned by {ORG}")
        return

    print(f"{repo}: transferring {OLD_OWNER}/{repo} -> {ORG}/{repo}")
    request("POST", f"/repos/{OLD_OWNER}/{repo}/transfer", {"new_owner": ORG}, ok=(202,))

    for _ in range(60):
        if repo_exists(ORG, repo):
            print(f"{repo}: transfer visible")
            return
        time.sleep(2)
    raise RuntimeError(f"{repo}: transfer did not become visible in time")


def set_runner_group_access(repo):
    _, data = request("GET", f"/repos/{ORG}/{repo}", ok=(200,))
    repo_id = int(data["id"])
    print(f"{repo}: allowing runner group {RUNNER_GROUP_ID}")
    request(
        "PUT",
        f"/orgs/{ORG}/actions/runner-groups/{RUNNER_GROUP_ID}/repositories/{repo_id}",
        None,
        ok=(204,),
    )


def update_workflow(repo, workflow_path):
    encoded = urllib.parse.quote(workflow_path, safe="")
    _, current = request("GET", f"/repos/{ORG}/{repo}/contents/{encoded}?ref=main", ok=(200,))
    content = base64.b64decode(current["content"]).decode("utf-8")
    updated = content.replace(
        "runs-on: [self-hosted, linux, x64, codex]",
        'runs-on:\n      group: "Home Server Runners"\n      labels: codex',
    )
    updated = updated.replace("max-parallel: 4", "max-parallel: 8")

    if updated == content:
        print(f"{repo}: {workflow_path} already up to date")
        return

    payload = {
        "message": MESSAGE,
        "content": base64.b64encode(updated.encode("utf-8")).decode("ascii"),
        "sha": current["sha"],
        "branch": "main",
    }
    request("PUT", f"/repos/{ORG}/{repo}/contents/{encoded}", payload, ok=(200, 201))
    print(f"{repo}: updated {workflow_path}")


def main():
    request("GET", f"/orgs/{ORG}", ok=(200,))
    request("GET", f"/orgs/{ORG}/actions/runner-groups/{RUNNER_GROUP_ID}", ok=(200,))
    if RUNNER_GROUP_SCOPE not in {"all", "selected"}:
        raise SystemExit("RUNNER_GROUP_SCOPE must be 'all' or 'selected'")
    if RUNNER_GROUP_SCOPE == "all":
        print("Runner group is treated as organization-wide; skipping per-repository access grants.")

    for repo in REPOS:
        transfer_repo(repo)
        if RUNNER_GROUP_SCOPE == "selected":
            set_runner_group_access(repo)
        for workflow_path in WORKFLOW_PATHS:
            update_workflow(repo, workflow_path)


if __name__ == "__main__":
    main()
PY
