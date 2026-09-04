---
name: home-server-ops-rollout
description: Plan, implement, and validate local operations work on dongwonttuna's Ubuntu home server. Use for MCP setup, systemd user services and timers, local service adoption, GitHub or Forgejo mirror automation, runner maintenance, shell-script hardening, host-grounded deployment plans, and requests that must verify live machine state before changing infrastructure.
---

# Home Server Ops Rollout

## Workflow

1. Inspect the live host before planning or changing anything.
   - Check the current working directory, repo state, relevant configs, live ports, service/timer status, logs, installed tools, and current versions.
   - For Codex or MCP work, check `codex mcp list`, Node/npm availability, package `latest` metadata, and the actual service state.
   - Answer for this machine first; avoid generic product guidance until local facts are known.

2. Use the user's operational defaults.
   - When the user asks for `latest`, use `@latest` in both the plan and implementation unless they override it.
   - Prefer local-first setups with no external exposure or API keys unless the request requires them.
   - Place automation in the repo the user names; do not leave production scripts in scratch paths.
   - Preserve the existing `origin` remote unless the user explicitly asks to change it.

3. Build ops automation defensively.
   - For mirror automation, include create-if-missing behavior for target repos unless told otherwise.
   - Avoid destructive defaults: do not prune remote-only refs unless the user opts in.
   - Preserve Forgejo PR refs by mapping them to explicit GitHub branch refs when mirroring.
   - Distinguish "repo missing" from auth, network, or permission failures before auto-creating anything.

4. Handle systemd and shell changes safely.
   - Stop active fast timers before editing the scripts they run, then re-enable and verify afterward.
   - Validate shell scripts with syntax checks such as `bash -n`.
   - Prefer dry runs before real syncs or deploys when the script supports them.
   - Verify user services and timers with `systemctl --user status`, `systemctl --user list-timers`, and `journalctl --user -u ...`.

5. Prove the rollout with evidence.
   - Run a smoke test that exercises the real path, not just config parsing.
   - For mirrors, verify remote refs with `git ls-remote` while tolerating extra GitHub-side refs.
   - For MCP/local services, verify health endpoints or a remember/search/cleanup-style smoke test when available.
   - Summarize exact commands run, important log lines, and any rollback hatch.

## Output

Keep the final report operational:

- Current host facts checked
- Files or services changed
- Validation commands and results
- Remaining risks, follow-up timers, or rollback steps
