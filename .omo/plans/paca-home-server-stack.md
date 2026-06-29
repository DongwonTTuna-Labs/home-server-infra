# paca-home-server-stack - Work Plan

## TL;DR (For humans)

**What you'll get:** Paca will be managed from the home-server infrastructure repo, reachable at `paca.dongwontuna.net`, connected to the existing local MCP suite, backed by its current data, and opened as a verified PR.

**Why this approach:** The plan keeps the current Paca data by preserving the same Compose project and volumes, while moving configuration into version control and using Cloudflare Tunnel as the only public entry point. You explicitly chose automatic updates for both the AI agent and Postgres, so the plan adds backup, drift detection, and rollback evidence around that higher-risk policy.

**What it will NOT do:** It will not expose MCP endpoints publicly, commit secrets or runtime data, merge the PR, or overwrite unrelated dirty worktree changes.

**Effort:** Large
**Risk:** High - live data, public ingress, secret rotation, Docker networking, and user-approved Watchtower auto-updates for `ai-agent` and Postgres.
**Decisions to sanity-check:** Only Paca `ai-agent` and Postgres are Watchtower-enabled despite the risks; Paca gateway is loopback-only behind Cloudflare; MCPs stay internal as per-agent Paca MCP rows over `paca_mcp_internal` only.

Your next move after this optional high-accuracy review passes: say `$start-work` to hand this plan to an executor. Full execution detail follows below.

---

> TL;DR (machine): Large/high-risk infra migration plan: add repo-managed `stacks/paca`, Cloudflare ingress, per-agent MCP rows over `paca_mcp_internal`, relay/xhigh enforcement, limited Watchtower auto-update exceptions, backup/secret rotation with ENCRYPTION_KEY re-encryption, live smoke, rollback, and PR.

## Scope
### Must have
- Add a repo-managed Paca stack under `stacks/paca/`.
- Preserve/reuse the existing Paca Compose project name `paca` and existing `paca_*` Docker volumes unless a preflight check proves they are missing.
- Configure Paca for `https://paca.dongwontuna.net` with secure cookies, CORS, and storage public URL.
- Bind Paca gateway to host loopback only, with Cloudflare Tunnel as the public ingress.
- Add `paca.dongwontuna.net` to the existing `stacks/tunnel-apps` tunnel before the catch-all rule.
- Keep all MCP endpoints off public Cloudflare ingress.
- Make existing MCPs (`lsp`, `codegraph`, `agbrowse`) reachable by Paca `ai-agent` and sandbox containers over the named internal Docker network `paca_mcp_internal`, using Paca per-agent MCP rows with URLs `http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, and `http://mcp-suite:8303/mcp`.
- Enforce Paca agent LLM settings to `ai-relay`, `gpt-5.5`, relay base URL, and runtime `reasoning_effort=xhigh`.
- Rotate all Paca runtime secrets because they were exposed in prior tool output; never print the values in logs/PR/evidence. `ENCRYPTION_KEY` must be rotated only through a transactional decrypt/re-encrypt migration for existing `agents.llm_api_key_secret` values; blindly changing it is forbidden because ai-agent returns an empty LLM key on decrypt failure.
- Label Paca `ai-agent` for Watchtower auto-update as explicitly approved by the user, with post-update xhigh drift detection and rollback docs.
- Label Paca Postgres for Watchtower auto-update as explicitly approved by the user, with pre-update backup, DB health, restore, and rollback docs.
- Update repo verification, secret inventory, restore docs, stack README, tunnel docs, and maintenance docs.
- Create a PR after all verification passes, without merging it.

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Must not commit `.env`, generated secrets, DB dumps, logs, `.omo/evidence`, backups, or runtime volumes.
- Must not print secret values from `/home/dongwonttuna/paca/.env`, container env, DB rows, or generated `.env` files.
- Must not blindly rotate `ENCRYPTION_KEY`; either every encrypted agent LLM key is re-encrypted from old key to new key in a verified transaction, or the cutover stops before changing `ENCRYPTION_KEY` and reports that agent LLM keys must be re-entered.
- Must not expose ports `8301`, `8302`, `8303`, `mcp-suite`, or `/mcp` through `stacks/tunnel-apps/cloudflared/tunnel-apps.yml`.
- Must not run `docker compose down -v`, `docker volume rm`, or destructive DB restore/migration without a verified backup and explicit user approval for the destructive action.
- Must not modify, stage, or overwrite unrelated dirty files in the existing worktree.
- Must not merge the PR.
- Must not claim Watchtower hooks can block a bad update; hooks can only detect/log, so rollback instructions and evidence are required.
- Must not add Watchtower enable labels to Paca `api`, `web`, `realtime`, `gateway`, `minio`, `valkey`, or `db-backup` in this plan; only the explicitly approved exceptions `ai-agent` and `postgres` may be `com.centurylinklabs.watchtower.enable=true`.

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after + shell/Docker/Cloudflare/Paca live smoke checks.
- Evidence: local-only files under `.omo/evidence/task-<N>-paca-home-server-stack.*`; these are ignored and must be summarized in the PR body, not committed.
- Required global gates before PR:
  - `scripts/scan-secrets.sh`
  - `scripts/verify-layout.sh`
  - `docker compose -f stacks/tunnel-apps/compose.yaml config --quiet`
  - `cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate`
  - Paca compose config with placeholder `.env.verify`, redirecting output to `/dev/null`.
  - Local health: `curl -fsS http://127.0.0.1:3080/api/healthz` returns `{"status":"ok"}`.
  - Public health: `curl -fsS https://paca.dongwontuna.net/api/healthz` returns `{"status":"ok"}`.
  - DB health: `docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres pg_isready ...` with no password printed.
  - xhigh smoke: import/check the mounted Paca `builder.py` in `ai-agent` and assert `reasoning_effort == "xhigh"` without printing API keys.
  - Encryption-key smoke: before `ENCRYPTION_KEY` rotation, count non-empty `agents.llm_api_key_secret` rows; run a local-only transaction that decrypts each with the old key and re-encrypts with the new key without printing plaintext; abort if decrypt-failure count is nonzero; after rotation, ai-agent decrypt smoke must return pass/boolean only.
  - MCP smoke: verify Paca `ai-agent` on `paca_mcp_internal` can POST MCP `initialize` to `http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, and `http://mcp-suite:8303/mcp` with HTTP 200/202 and no DNS/connection errors; also verify tunnel config still rejects MCP exposure.
  - Backup smoke: create or verify a current Postgres backup artifact and prove it is non-empty before any cutover/update-risk step.

## Execution strategy
### Parallel execution waves
- Wave 0: Worktree isolation and preflight inventory.
- Wave 1: Repo-managed Paca files, docs, verification script, and tunnel docs in parallel after the clean worktree exists.
- Wave 2: Compose/network/secret/backup validation after files exist.
- Wave 3: Live cutover and smoke after config validation and backup.
- Wave 4: PR preparation after all verification passes.

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 1 | none | all implementation todos | none |
| 2 | 1 | 5, 7, 8, 10 | 3, 4, 6, 9 |
| 3 | 1 | 8, 10 | 2, 4, 6, 9 |
| 4 | 1 | 8, 10 | 2, 3, 6, 9 |
| 5 | 2 | 8, 10 | 6, 7, 9 |
| 6 | 1 | 8, 10 | 2, 3, 4, 5, 7 |
| 7 | 2 | 8, 10 | 5, 6, 9 |
| 8 | 2, 3, 4, 5, 6, 7 | 10, 11 | 9 |
| 9 | 1 | 10, 11 | 2, 3, 4, 5, 6, 7, 8 |
| 10 | 8, 9 | 11, 12 | none |
| 11 | 10 | 12 | none |
| 12 | 11 | final verification | none |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [x] 1. Worktree: create isolated implementation branch to protect dirty repo state
  What to do / Must NOT do: Create a sibling git worktree from the remote tracking branch, e.g. `../home-server-infra-paca-stack`, on branch `feat/paca-home-server-stack`. Prefix git history/status commands with `GIT_MASTER=1`. Do not stage or edit existing dirty files in `/home/dongwonttuna/Documents/Programming/home-server-infra`.
  Parallelization: Wave 0 | Blocked by: none | Blocks: 2-12
  References (executor has NO interview context - be exhaustive): `.omo/drafts/paca-home-server-stack.md:54-55,90`; `README.md:14-15`; developer git rules require status/diff/log before commit.
  Acceptance criteria (agent-executable): In the new worktree, `GIT_MASTER=1 git status --short` shows no unrelated dirty files before edits; original worktree status is unchanged except `.omo` plan artifacts.
  QA scenarios (name the exact tool + invocation): happy: `GIT_MASTER=1 git worktree list` includes the new path and branch; failure: `GIT_MASTER=1 git status --short` in original worktree still lists the same unrelated dirty files and no implementation files staged. Evidence `.omo/evidence/task-1-paca-home-server-stack.txt`.
  Commit: N | setup-only, no product change

- [x] 2. `stacks/paca`: add repo-managed Compose, Caddy, env example, SQL, and override files
  What to do / Must NOT do: Create `stacks/paca/compose.yaml`, `stacks/paca/caddy/Caddyfile`, `stacks/paca/.env.example`, `stacks/paca/relay-ai-enforce.sql`, `stacks/paca/mcp-local-servers.sql`, `stacks/paca/docker-compose.override.yaml`, and `stacks/paca/overrides/ai-agent/builder.py`. Base them on the live Paca files but remove secrets. Keep `name: paca`; set the Compose default network to the explicit name `paca_mcp_internal` so Paca `ai-agent` and its spawned sandbox join the same network as `mcp-suite`; set gateway ports to loopback defaults (`127.0.0.1:3080:80`, optionally `127.0.0.1:3443:443`); set `SITE_ADDRESS=:80`; set public URL defaults for `https://paca.dongwontuna.net`; set Watchtower enable labels only for the two explicitly approved exceptions: `ai-agent` and `postgres`; omit or set false for `api`, `web`, `realtime`, `gateway`, `minio`, `valkey`, and `db-backup`. Document that Postgres/ai-agent labels are explicit exceptions and that `ENCRYPTION_KEY` requires re-encryption migration. Do not commit `.env`.
  Parallelization: Wave 1 | Blocked by: 1 | Blocks: 5, 7, 8, 10
  References (executor has NO interview context - be exhaustive): `/home/dongwonttuna/paca/docker-compose.yml:24-34,48,53-68,69-80,127-131,170-226,251-318`; `/home/dongwonttuna/paca/caddy/Caddyfile:5-13,16-34,60-84,121-136`; `/home/dongwonttuna/paca/docker-compose.override.yml:1-5`; `/home/dongwonttuna/paca/relay-ai-enforce.sql:1-34`; `/home/dongwonttuna/paca/overrides/ai-agent/builder.py:16-45`; `.omo/drafts/paca-home-server-stack.md:47-53,66-73`.
  Acceptance criteria (agent-executable): `test -f` passes for all listed files; `grep -q '^name: paca$' stacks/paca/compose.yaml`; `grep -q 'name: paca_mcp_internal' stacks/paca/compose.yaml`; `grep -q '127.0.0.1:3080' stacks/paca/compose.yaml`; Watchtower true labels exist only for `ai-agent` and `postgres` (not `api`, `web`, `realtime`, `gateway`, `minio`, `valkey`, or `db-backup`); `test ! -f stacks/paca/.env`; `scripts/scan-secrets.sh` passes.
  QA scenarios (name the exact tool + invocation): happy: create `stacks/paca/.env.verify` with placeholder-only values and run `docker compose --env-file stacks/paca/.env.verify -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml config --quiet`; failure: `grep -R "changeme\|minioadmin\|POSTGRES_PASSWORD=.*[^}]" stacks/paca --exclude=.env.example` must not reveal real secrets. Evidence `.omo/evidence/task-2-paca-home-server-stack.txt`.
  Commit: Y | `feat(paca): add repo-managed paca stack`

- [x] 3. Tunnel apps: add `paca.dongwontuna.net` ingress without exposing MCPs
  What to do / Must NOT do: Update `stacks/tunnel-apps/cloudflared/tunnel-apps.yml` to add `hostname: paca.dongwontuna.net` with `service: http://localhost:3080` before the `http_status:404` catch-all. Update `stacks/tunnel-apps/README.md` to document the route. Do not add `8301`, `8302`, `8303`, `mcp-suite`, or `/mcp` anywhere in tunnel config.
  Parallelization: Wave 1 | Blocked by: 1 | Blocks: 8, 10
  References (executor has NO interview context - be exhaustive): `stacks/tunnel-apps/cloudflared/tunnel-apps.yml:4-9`; `stacks/tunnel-apps/README.md:3-13`; `stacks/tunnel-apps/compose.yaml:4-16`; `scripts/verify-layout.sh:88-90`; `.omo/drafts/paca-home-server-stack.md:39-46,77-80,88`.
  Acceptance criteria (agent-executable): `cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate` exits 0; `grep -q 'paca.dongwontuna.net' stacks/tunnel-apps/cloudflared/tunnel-apps.yml`; MCP forbidden grep still returns no match.
  QA scenarios (name the exact tool + invocation): happy: `cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate`; failure: `! grep -Eq '8301|8302|8303|mcp-suite|/mcp' stacks/tunnel-apps/cloudflared/tunnel-apps.yml`. Evidence `.omo/evidence/task-3-paca-home-server-stack.txt`.
  Commit: Y | `feat(tunnel-apps): route paca hostname`

- [x] 4. MCP suite network: add internal network path for Paca without public exposure
  What to do / Must NOT do: Update `stacks/mcp-suite/compose.yaml` so service `mcp-suite` keeps its existing default network and also joins the external Docker network `paca_mcp_internal` with alias `mcp-suite`. The exact Compose shape must include `networks: { paca_mcp_internal: { external: true, name: paca_mcp_internal } }` and service-level `networks: [default, paca_mcp_internal]` or equivalent alias syntax. Paca creates `paca_mcp_internal` as its default network in `stacks/paca/compose.yaml`; therefore live attach/restart of `mcp-suite` happens after Paca creates the network. Preserve host-side loopback-only port publishing. Update `stacks/mcp-suite/README.md` to document the Paca internal URL forms: `http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, `http://mcp-suite:8303/mcp`. Do not expose these through Cloudflare.
  Parallelization: Wave 1 | Blocked by: 1 | Blocks: 8, 10
  References (executor has NO interview context - be exhaustive): `stacks/mcp-suite/README.md:7-16,26-32,39-44`; `stacks/mcp-suite/compose.yaml` existing loopback ports; `/tmp/opencode/paca-src/services/ai-agent/src/agent/docker_workspace.py:180-185,279-285`; `.omo/drafts/paca-home-server-stack.md:45-46,53,56-57,80,88`.
  Acceptance criteria (agent-executable): `docker compose -f stacks/mcp-suite/compose.yaml config --quiet` passes; `grep -q '127.0.0.1:8301' stacks/mcp-suite/compose.yaml`; `grep -q 'paca_mcp_internal' stacks/mcp-suite/compose.yaml`; README documents internal Paca URLs; tunnel forbidden grep still passes.
  QA scenarios (name the exact tool + invocation): happy: `docker compose -f stacks/mcp-suite/compose.yaml config --quiet`; failure: `! grep -Eq '8301|8302|8303|mcp-suite|/mcp' stacks/tunnel-apps/cloudflared/tunnel-apps.yml`. Evidence `.omo/evidence/task-4-paca-home-server-stack.txt`.
  Commit: Y | `feat(mcp-suite): add paca internal network path`

- [x] 5. Paca ai-agent and MCP seed: force xhigh and register local MCPs as per-agent rows
  What to do / Must NOT do: In `stacks/paca/overrides/ai-agent/builder.py`, retain only the relay `reasoning_effort="xhigh"` behavior and preserve upstream DB MCP loading plus built-in Paca MCP ordering semantics. Do not hide `lsp`, `codegraph`, or `agbrowse` as global builder defaults. Instead, implement `stacks/paca/mcp-local-servers.sql` as an idempotent seed/upsert for enabled per-agent MCP rows named `lsp`, `codegraph`, and `agbrowse` with `transport='http'`, no command/args/env secrets, and URLs `http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, `http://mcp-suite:8303/mcp` for every existing active Paca agent at cutover time. Future agents must add these same rows via documented Paca UI/API/SQL procedure. The seed must preserve user-configured MCP rows not named by this plan and must be safe to re-run.
  Parallelization: Wave 2 | Blocked by: 2 | Blocks: 8, 10
  References (executor has NO interview context - be exhaustive): `/tmp/opencode/paca-src/services/ai-agent/src/agent/builder.py:18-50,65-109`; `/tmp/opencode/paca-src/services/ai-agent/tests/test_builder.py:201-270`; `/tmp/opencode/paca-src/services/api/migrations/000008_add_ai_agents.sql:57-76`; `/home/dongwonttuna/paca/overrides/ai-agent/builder.py:16-45`; `stacks/mcp-suite/README.md:9-13`; `.omo/drafts/paca-home-server-stack.md:31,56-58,72`.
  Acceptance criteria (agent-executable): A local Python syntax/behavior smoke imports the override and asserts the constructed LLM receives `reasoning_effort="xhigh"` without printing keys; SQL/static smoke verifies `mcp-local-servers.sql` upserts `lsp`, `codegraph`, and `agbrowse` URLs using `mcp-suite` (never `127.0.0.1`) and does not delete unrelated MCP rows; builder override grep confirms it does not hard-code the local custom MCP names.
  QA scenarios (name the exact tool + invocation): happy: run a small `python - <<'PY' ... PY` smoke against `stacks/paca/overrides/ai-agent/builder.py` with fake settings and fake rows, plus `grep -q 'http://mcp-suite:8301/mcp' stacks/paca/mcp-local-servers.sql`; failure: `! grep -R '127.0.0.1:830[123]\|lsp\|codegraph\|agbrowse' stacks/paca/overrides/ai-agent/builder.py` unless the grep hit is in a comment explicitly saying local MCPs must not be builder defaults. Evidence `.omo/evidence/task-5-paca-home-server-stack.txt`.
  Commit: Y | `feat(paca): enforce xhigh and local mcp rows`

- [x] 6. Docs: add Paca secret inventory, restore, backup, rollback, and Watchtower exception docs
  What to do / Must NOT do: Update `docs/secrets.md`, `docs/restore.md`, `stacks/paca/README.md`, and `stacks/maintenance/README.md`. Include Paca secret names only, never values. Include volume names, backup paths, restore commands, safe start/stop commands, the `paca_mcp_internal` network, the `mcp-local-servers.sql` seed procedure, Watchtower label matrix, and explicit warning that only `ai-agent`/Postgres auto-updates are user-approved high-risk exceptions. Document `ENCRYPTION_KEY` separately: it protects encrypted agent LLM keys, must be 64 hex chars, and may only be rotated by decrypt/re-encrypt migration or by intentionally stopping for manual LLM-key re-entry; never recommend blindly changing it. State `.omo/evidence` is local-only and ignored.
  Parallelization: Wave 1 | Blocked by: 1 | Blocks: 8, 10
  References (executor has NO interview context - be exhaustive): `docs/secrets.md:1-5,6-33,42-53`; `docs/restore.md:1-67`; `stacks/maintenance/README.md:6-20`; `stacks/maintenance/compose.yaml:11-16`; `.gitignore:1-27,41-54,77-85`; `/home/dongwonttuna/paca/docker-compose.yml:69-78,127-131,181-206,287-306,318-326`; `.omo/drafts/paca-home-server-stack.md:60-63,70-73`.
  Acceptance criteria (agent-executable): `grep -q 'Paca' docs/secrets.md docs/restore.md stacks/maintenance/README.md stacks/paca/README.md`; docs mention `ENCRYPTION_KEY` re-encryption and `paca_mcp_internal`; `scripts/scan-secrets.sh` passes; docs contain no real secret-looking values.
  QA scenarios (name the exact tool + invocation): happy: `scripts/scan-secrets.sh`; failure: `! git grep -nE '(POSTGRES_PASSWORD|JWT_SECRET|AGENT_API_KEY|INTERNAL_API_KEY|ENCRYPTION_KEY)=.{8,}' -- ':!**/.env.example'`. Evidence `.omo/evidence/task-6-paca-home-server-stack.txt`.
  Commit: Y | `docs(paca): document secrets restore and update policy`

- [x] 7. Verification script: extend layout checks for Paca stack, tunnel, and secret hygiene
  What to do / Must NOT do: Update `scripts/verify-layout.sh` to require Paca stack files, run Paca compose config using a generated placeholder `.env.verify` in a temp directory, assert `name: paca`, assert network name `paca_mcp_internal`, assert gateway loopback binding, assert Watchtower true labels include only `ai-agent` and `postgres`, assert `api`/`web`/`realtime`/`gateway`/`minio`/`valkey`/`db-backup` are not Watchtower-enabled, assert MCP seed URLs use `http://mcp-suite:8301-8303/mcp`, and preserve the existing MCP tunnel exposure ban. Do not require a real `.env` or secret.
  Parallelization: Wave 2 | Blocked by: 2 | Blocks: 8, 10
  References (executor has NO interview context - be exhaustive): `scripts/verify-layout.sh:6-24,50-53,88-90,116-128`; `.gitignore:84`; `.omo/drafts/paca-home-server-stack.md:63,68`.
  Acceptance criteria (agent-executable): `scripts/verify-layout.sh` exits 0 from the implementation worktree; it fails if a temporary copy removes `stacks/paca/compose.yaml`, if tunnel config contains MCP exposure, if Paca compose lacks `paca_mcp_internal`, if `minio`/`valkey`/`db-backup` are Watchtower-enabled, or if MCP seed URLs use `127.0.0.1`.
  QA scenarios (name the exact tool + invocation): happy: `scripts/verify-layout.sh`; failure: run a temp-copy negative check that injects `8301` into a copied tunnel config and confirms the script logic would reject it. Evidence `.omo/evidence/task-7-paca-home-server-stack.txt`.
  Commit: Y | `test(paca): verify stack layout and tunnel guardrails`

- [x] 8. Static validation: prove repo config is secret-free and compose-valid before live changes
  What to do / Must NOT do: Run static validation only. Do not start/stop containers yet. Capture non-secret command outputs to `.omo/evidence`. If compose config output may include placeholders, redirect it to `/dev/null` and record only pass/fail.
  Parallelization: Wave 2 | Blocked by: 2, 3, 4, 5, 6, 7 | Blocks: 10, 11
  References (executor has NO interview context - be exhaustive): `README.md:41-54`; `scripts/scan-secrets.sh:22-33`; `scripts/verify-layout.sh:50-53,116-128`; `.omo/drafts/paca-home-server-stack.md:71`.
  Acceptance criteria (agent-executable): all static gates pass: `scripts/scan-secrets.sh`, `scripts/verify-layout.sh`, `docker compose -f stacks/tunnel-apps/compose.yaml config --quiet`, `docker compose -f stacks/mcp-suite/compose.yaml config --quiet`, Paca compose config with placeholder env, and Cloudflare ingress validation. Static evidence also proves the rendered Paca network is named `paca_mcp_internal`, only `ai-agent`/`postgres` are Watchtower-enabled, and MCP seed URLs use `mcp-suite` DNS.
  QA scenarios (name the exact tool + invocation): happy: run all commands above and save exit statuses; failure: `git grep -nE 'POSTGRES_PASSWORD=.*[^}]|JWT_SECRET=.*[^}]|AGENT_API_KEY=.*[^}]' -- ':!**/.env.example'` returns no tracked real values. Evidence `.omo/evidence/task-8-paca-home-server-stack.txt`.
  Commit: N | verification only

- [x] 9. Preflight live inventory: record non-secret current Paca state, backup readiness, and encrypted-secret migration scope
  What to do / Must NOT do: Read only Docker metadata and health. Record current Paca container image refs/digests, volume names, network names, exposed host ports, and health endpoints. Do not print env variables or secret values. Verify current backup service or create a safe fresh Postgres backup through the existing backup mechanism before cutover; do not restore anything. Query only counts/metadata for encrypted agent secrets: count active agents, count non-empty `agents.llm_api_key_secret` values, and count values that appear encrypted (`enc:` prefix or AES-GCM/base64 format per current Paca code) without selecting or printing the secret column values.
  Parallelization: Wave 1/2 | Blocked by: 1 | Blocks: 10, 11
  References (executor has NO interview context - be exhaustive): `/home/dongwonttuna/paca/docker-compose.yml:69-78,127-131,318-326`; `.omo/drafts/paca-home-server-stack.md:47-53,70,73`; Metis gap on backup/rollback.
  Acceptance criteria (agent-executable): evidence includes non-secret image digest list, volume list including `paca_postgres_data`, backup artifact path/size/timestamp, `curl -fsS http://127.0.0.1:3080/api/healthz` current status, and encrypted-secret scope counts only (no `llm_api_key_secret` values).
  QA scenarios (name the exact tool + invocation): happy: `docker compose --env-file /home/dongwonttuna/paca/.env -f /home/dongwonttuna/paca/docker-compose.yml -f /home/dongwonttuna/paca/docker-compose.override.yml ps` plus safe `docker inspect --format` without env; failure: no command may include `docker inspect ... .Config.Env` or `docker compose config` against the real `.env` without redirecting sensitive output. Evidence `.omo/evidence/task-9-paca-home-server-stack.txt`.
  Commit: N | live preflight only

- [x] 10. Live cutover: start repo-managed Paca with rotated secrets, ENCRYPTION_KEY re-encryption, and health smoke
  What to do / Must NOT do: After backup evidence exists, create/rotate `stacks/paca/.env` locally from `.env.example` without printing values. For ordinary runtime secrets, generate new values. For `ENCRYPTION_KEY`, generate a new 64-hex value but do not activate it until the worker runs a local-only transactional re-encryption: using old and new keys supplied only via process env or root-readable temp files, decrypt every non-empty `agents.llm_api_key_secret`, re-encrypt with the new key, update rows in one transaction, verify zero decrypt failures and matching row count, then remove temp files. If any decrypt fails, abort before changing `ENCRYPTION_KEY` and report that affected agent LLM keys must be manually re-entered; do not preserve the compromised key as the final state. Stop/recreate using repo-managed Compose while preserving `name: paca` volumes. Do not use `down -v`. Bring up `stacks/paca/compose.yaml` plus override, then restart/attach `mcp-suite` after `paca_mcp_internal` exists. Restart tunnel-apps if needed. Smoke local and public health, cookies/CORS/storage URL behavior, DB health, and that gateway binds only loopback.
  Parallelization: Wave 3 | Blocked by: 8, 9 | Blocks: 11, 12
  References (executor has NO interview context - be exhaustive): `/home/dongwonttuna/paca/docker-compose.yml:48,181-206,251-263,287-306,318-326`; `/home/dongwonttuna/paca/caddy/Caddyfile:37-84,121-136`; `stacks/tunnel-apps/compose.yaml:4-16`; `.omo/drafts/paca-home-server-stack.md:25-33,48-52,78-84`.
  Acceptance criteria (agent-executable): `docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml ps` shows expected services healthy/running; `curl -fsS http://127.0.0.1:3080/api/healthz` and `curl -fsS https://paca.dongwontuna.net/api/healthz` return OK; `ss -ltnp` or Docker ps shows no `0.0.0.0:3080`; DB `pg_isready` passes; encrypted LLM key re-encryption count matches preflight count or preflight count is zero; secrets are not printed.
  QA scenarios (name the exact tool + invocation): happy: local/public curls, DB health, encrypted-secret re-key smoke with pass/count-only output, `docker compose ps`; failure: `! docker ps --format '{{.Ports}}' | grep -E '0\.0\.0\.0:3080|:::3080'` and a negative dry run that exits nonzero if decrypt-failure count is nonzero. Evidence `.omo/evidence/task-10-paca-home-server-stack.txt`.
  Commit: N | live cutover only

- [x] 11. Runtime policy smoke: verify relay/xhigh, MCP reachability, Watchtower labels, encryption, backup, and rollback docs
  What to do / Must NOT do: Verify DB trigger/check constraint, all active agents are `ai-relay`/`gpt-5.5`/relay base URL, ai-agent can decrypt re-encrypted agent LLM keys with the new `ENCRYPTION_KEY`, runtime builder uses `reasoning_effort=xhigh`, Paca can reach MCP internal URLs, Watchtower labels exist only for `ai-agent` and `postgres`, and rollback docs include prior image digests and DB restore path. Do not print LLM API keys, plaintext decrypted values, old/new `ENCRYPTION_KEY`, or secret env.
  Parallelization: Wave 3 | Blocked by: 10 | Blocks: 12
  References (executor has NO interview context - be exhaustive): `/home/dongwonttuna/paca/relay-ai-enforce.sql:1-34`; `/tmp/opencode/paca-src/services/api/migrations/000008_add_ai_agents.sql:57-76`; `/tmp/opencode/paca-src/services/api/internal/platform/secret/encryptor.go:51-71`; `/tmp/opencode/paca-src/services/ai-agent/src/repositories/agent_repository.py:16-51,122-141`; `/tmp/opencode/paca-src/services/ai-agent/src/agent/builder.py:45-50,65-109`; `/tmp/opencode/paca-src/services/ai-agent/src/agent/docker_workspace.py:180-185,279-285`; `stacks/maintenance/compose.yaml:11-16`; `.omo/drafts/paca-home-server-stack.md:56-58,72-73`.
  Acceptance criteria (agent-executable): SQL checks return zero violating rows; encryption smoke returns `decrypt_ok=true` and count-only output for all non-empty agent LLM key rows; xhigh smoke returns a boolean/pass line only; MCP smoke from Paca `ai-agent` can initialize all three internal URLs; Docker inspect labels for `paca-ai-agent-1` and `paca-postgres-1` include Watchtower enable true and labels for `paca-minio-1`, `paca-valkey-1`, and `paca-db-backup-1` are absent/false; backup artifact remains present and non-empty.
  QA scenarios (name the exact tool + invocation): happy: `docker compose exec -T postgres psql ... -c 'SELECT count(*) ...'` with password supplied via env but not printed; xhigh Python smoke inside `ai-agent`; Paca-side MCP smoke: `docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml exec -T ai-agent python - <<'PY'` that POSTs MCP `initialize` JSON to `http://mcp-suite:8301/mcp`, `:8302/mcp`, `:8303/mcp` and asserts HTTP 200/202; failure: the same smoke with `http://127.0.0.1:8301/mcp` must fail from inside `ai-agent`, and tunnel grep must reject any MCP URL. Evidence `.omo/evidence/task-11-paca-home-server-stack.txt`.
  Commit: N | runtime verification only

- [ ] 12. PR: commit intended files only and open a verified PR
  What to do / Must NOT do: Before committing inspect `GIT_MASTER=1 git status`, `git diff`, and `git log --oneline -10`. Stage only intended repo files, not `.env`, backups, `.omo/evidence`, or unrelated dirty files. Run final gates. Commit with concise messages matching repo style. Push only after all verification passes. Open a PR with scope, risk, verification evidence summary, Watchtower exception warning, rollback summary, and note that PR is not merged.
  Parallelization: Wave 4 | Blocked by: 11 | Blocks: final verification
  References (executor has NO interview context - be exhaustive): global git instructions; `.gitignore:1-27,41-54,77-85`; `scripts/scan-secrets.sh:22-33`; `.omo/drafts/paca-home-server-stack.md:54-55,85,90-91`; Metis gap on clean worktree/PR.
  Acceptance criteria (agent-executable): PR URL exists; `GIT_MASTER=1 git status --short` is clean except ignored local runtime files; PR body includes verification commands and results; no secret/runtime paths are tracked; original dirty worktree remains untouched.
  QA scenarios (name the exact tool + invocation): happy: `scripts/scan-secrets.sh && scripts/verify-layout.sh && cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate`; failure: `GIT_MASTER=1 git diff --cached --name-only | grep -E '(^|/)(\.env|backups|logs|state|\.omo/evidence)'` returns no matches. Evidence `.omo/evidence/task-12-paca-home-server-stack.txt`.
  Commit: Y | `feat(paca): manage paca home server stack`

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit: read this plan and changed files; verify every Must Have/Must NOT is satisfied, especially no MCP tunnel exposure, no secrets, clean worktree isolation, and no PR merge.
- [ ] F2. Code/config quality review: review Compose, Caddy, SQL, override Python, shell verification changes, and docs for maintainability, idempotence, and rollback clarity.
- [ ] F3. Automated live QA: independently rerun static gates, local/public Paca health, DB health, ENCRYPTION_KEY decrypt/re-key smoke, xhigh smoke, Paca-side MCP smoke, Watchtower-label checks, and backup existence checks.
- [ ] F4. Scope fidelity/security review: verify no unrelated repo changes, no committed secrets/runtime files, no destructive Docker operations, no blind `ENCRYPTION_KEY` rotation, no public MCP exposure, and high-risk Watchtower exceptions are limited to `ai-agent`/Postgres and explicitly documented.

## Commit strategy
- Use the isolated worktree branch `feat/paca-home-server-stack`.
- Prefer one final squashed feature commit unless intermediate commits already exist cleanly; acceptable commit message: `feat(paca): manage paca home server stack`.
- Before committing, inspect `GIT_MASTER=1 git status`, `GIT_MASTER=1 git diff`, and `GIT_MASTER=1 git log --oneline -10`.
- Stage only intended files: `.gitignore` if needed to keep `.omo/evidence` and `.omo/notepads` ignored, `stacks/paca/**`, `stacks/tunnel-apps/**`, `stacks/mcp-suite/**`, `stacks/maintenance/README.md`, `docs/secrets.md`, `docs/restore.md`, `scripts/verify-layout.sh`, `tests/grimoire_labels_contract_test.py` if needed to keep `scripts/scan-secrets.sh` passing, and any plan file if intentionally included.
- Never stage `.env`, `.env.verify`, `.omo/evidence`, backups, logs, DB dumps, runtime state, or unrelated dirty files.
- Push only after all verification gates pass; open a PR with `gh pr create`; do not merge.

## Success criteria
- Paca is repo-managed under `stacks/paca` and still uses Compose project `paca`.
- `paca.dongwontuna.net` routes through `tunnel-apps` to loopback Paca gateway.
- Paca gateway no longer binds `0.0.0.0:3080`.
- Paca local and public health endpoints return OK.
- Existing Paca data/volumes are preserved and backup evidence exists.
- All Paca runtime secrets are rotated locally and no secret values are committed or printed; `ENCRYPTION_KEY` is rotated only after verified re-encryption of existing encrypted agent LLM keys, or cutover stops before changing it.
- Paca agents are enforced to relay AI GPT-5.5 and runtime xhigh.
- Paca can reach the existing MCP suite internally over `paca_mcp_internal`; MCPs are registered as per-agent Paca MCP rows and are not exposed through Cloudflare.
- Watchtower labels include only user-approved `ai-agent` and Postgres auto-update exceptions, with documented detection/rollback.
- `scripts/scan-secrets.sh`, `scripts/verify-layout.sh`, compose config checks, and Cloudflare ingress validation pass.
- PR is opened with verification evidence summary and is not merged.
