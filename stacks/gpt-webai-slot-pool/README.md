# gpt-webai-slot-pool

Docker-backed ChatGPT/GPT Pro slot pool for `gpt-webai-lifecycle`.

This stack exists because GPT Pro delegation needs live-only browser evidence:
authenticated composer readiness, real `/c/...` turn confirmation, durable
answer artifacts, and downloadable ChatGPT-rendered files. Production paths use
the in-repo Playwright provider. The lifecycle supervisor owns allocation,
session ledgers, attachment staging, duplicate-send prevention, and slot
recovery. The Docker stack only provides isolated browser runtimes.

## Architecture

- Ten explicit services: `gpt-webai-slot-01` through `gpt-webai-slot-10`
- One Chromium + Xvfb runtime per slot
- One persistent Chrome profile per slot:
  `/state/slot-N/browser-profile`
- One `BROWSER_AGENT_HOME` per slot:
  `/state/slot-N`
- One CDP port per slot inside the container:
  `9223` through `9232`
- One read-only attachment mount per slot:
  `/broker-attachments`
- One shared writable host-backed R13 artifact mount, visible in every slot:
  `/broker-artifacts`
- Current physical account cohorts: `slot-01..03=cohort-a`,
  `slot-04..07=cohort-b`, `slot-08..10=cohort-c`; new sends rotate across
  configured cohorts and within the selected cohort using persisted cursors,
  while `resume`/`show` stay pinned to the recorded `sessionId -> slotId` and
  cohort snapshot. Legacy `group-01/group-02` may remain deployment topology
  labels only; they are not account, fairness, cooldown, or authorization
  authority.
- No host-published ports in normal operation
- Access from automation is only through `gpt-webai-lifecycle` and
  `docker exec`

The normal entrypoints are:

```bash
gptpro "prompt"
gptxhigh "prompt"
gptpro --file /path/to/context.zip "prompt"
gpt-webai-lifecycle status --json
gpt-webai-lifecycle preflight --json --docker-slot-provider --slot slot-01 --run-id RUN_ID
gpt-webai-lifecycle show --json --session SESSION_ID --fencing-token INVOCATION_TOKEN --docker-slot-provider
gpt-webai-lifecycle resume --json --session SESSION_ID --fencing-token INVOCATION_TOKEN --docker-slot-provider
gpt-webai-lifecycle download --json --session SESSION_ID --fencing-token INVOCATION_TOKEN --artifact-expectation optional --docker-slot-provider
gpt-webai-lifecycle release --json --session SESSION_ID --fencing-token INVOCATION_TOKEN
gpt-webai-lifecycle cleanup --json --apply
```

`queue resume` and `browser ensure` are retired and unsupported operator
commands. Do not run them unless a newly approved design explicitly
reintroduces and implements them.

Do not use raw browser-agent commands, MCP `web_ai_*` tools, or ad hoc
Playwright scripts for ordinary GPT delegation. Those paths skip the slot
ledger, duplicate-send guardrails, artifact persistence, and release contract.

## Included Supervisor

This stack also vendors the lifecycle supervisor used by the slot broker:

```text
stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle
```

PR #72 moves lifecycle authority from the Bash supervisor into a Rust
supervisor plus Node.js Playwright provider:

- `stacks/gpt-webai-slot-pool/Cargo.toml` defines the Rust supervisor
  workspace.
- `stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle` is the operator
  entrypoint. It is a thin Bash exec shim into the Rust CLI.
- Rust status reconciles persisted slot records with Docker runtime state. A
  slot whose container is stopped or exited is not reported as `ready`; `ready`
  is reserved for a live runtime that can be checked as an authenticated Pro
  composer.
- The former long Bash supervisor is not the production dispatcher. Bash is
  limited to compatibility shims and scripts; production lifecycle decisions
  live in Rust and the ChatGPT UI operations live in the Node Playwright
  provider.

Install or refresh the operator copy with:

```bash
ln -sfn \
  "$PWD/stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle" \
  "$HOME/.local/bin/gpt-webai-lifecycle"
```

The shim resolves symlinks before locating `Cargo.toml`, so do not copy it to a
different directory unless you also provide the repo layout it expects.

The supervisor owns:

- ready-slot allocation and pool-busy/queued envelopes
- `sessionId -> slotId` pinning for resume/poll/show
- duplicate-send prevention by request fingerprint
- durable answer artifacts before the answer is printed to the caller
- send-start confirmation using a real non-root `https://chatgpt.com/c/...`
  conversation URL plus server-assigned `data-message-id` identities for both
  the new user and assistant turns before long polling. Active generation may
  appear before the URL or assistant identity hydrates, but all three proofs
  must be observed inside the bounded window; counts, content, indices,
  timestamps, and Stop controls are not identity fallbacks
- slot-specific `docker exec` execution
- direct Playwright provider calls through `gpt-webai-provider`
- ChatGPT Pro/model readiness checks before send; composer visibility alone is
  not sufficient. For a Pro request, an initial `Instant`, `Extra High`, or
  other model label is normal input to the default picker flow: capture the
  actual Chrome screenshot+DOM/CDP, open the picker, select visible/available
  `Pro`, then recapture and verify the composer label before upload/send.
  `model.selection_mismatch` is reserved for picker-proven absence or bounded
  selection/reverification failure, followed by healthy-cohort rotation or an
  evidenced `recovering`/`queued` envelope; silent downgrade is forbidden.
  The provider-level R13 absence reasons are `picker.model_absent` and
  `picker.effort_absent`; bounded failures use `picker.control_drift`,
  `picker.selection_timeout`, or `picker.reverify_mismatch`. Provider receipts
  name root capture `capture.root` and stale attachment evidence `ChipProof`;
  retained R5 `capture` and `StaleChipProof` are not current terms.
- slot lease release at the end of every supervised `run`, `resume`, or
  artifact `download`, including slot runtime shutdown after terminal answers
- attachment capsule staging into `/broker-attachments`
- provider-side attachment chip/upload-complete confirmation before send
- ChatGPT-rendered artifact downloads into host-backed `/broker-artifacts`
  evidence directories; URLs are debug-only, while saved path, size, SHA-256,
  visible element, sessionId, and `/c/...` URL are durable truth
- compound extension preservation for files such as `.tar.gz`
- `ATTACHMENT_ACCESS_GATE` prompts and `provider.attachment_unavailable`
  recovery envelopes
- ChatGPT login-state gating; a visible login/signup UI makes the slot
  `auth.needs_login`/`reseed_login`, not `ready`
- ChatGPT provider-limit gating; "too many requests", rate-limit, or message
  cap UI makes the slot `provider.limit`/`degraded`, not `ready`
- provider-limit fallback tries distinct healthy cohorts in deterministic
  order; if all configured cohorts report provider limits, the lifecycle uses
  the approved bounded cooldown budget before retrying the three-cohort
  sequence. After a cooldown, persisted `provider.limit` slots are only
  reopened for fresh screenshot/DOM/send verification, not counted as
  recovered.
- persisted `provider.limit` slot state records an observed timestamp and a
  3-minute `next_retry_at`; after that TTL, stopped slots become standby
  candidates for the next real provider check while still preserving
  `persisted_status=provider.limit` until fresh evidence proves recovery

Offline regression tests live under:

```text
stacks/gpt-webai-slot-pool/tests/gpt-webai-lifecycle
```

The repo copy of the operator runbook is:

```text
stacks/gpt-webai-slot-pool/docs/gpt-webai-lifecycle-runbook.md
```

If the active Codex runbook needs to be refreshed from this repo, copy it to
`$HOME/.codex/runbooks/gpt-webai-lifecycle.md`.

## Bootstrap

Let the lifecycle supervisor create the host state directories with the same
UID/GID that the containers will run as:

```bash
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-lifecycle/r13"

GPT_WEBAI_SLOT_MODE=on GPT_WEBAI_SLOT_COUNT=10 gpt-webai-lifecycle status

find "$STATE" -maxdepth 2 \( -name evidence -o -name attachments -o -name prompts -o -name artifacts \) -type d -print

export GPT_WEBAI_SLOT_UID="$(id -u)"
export GPT_WEBAI_SLOT_GID="$(id -g)"
export GPT_WEBAI_STATE_ROOT="$STATE"
```

Validate and start the stack:

```bash
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml config
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml up -d --build
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml ps
```

Container health only proves Chromium/CDP is reachable. It does not prove the
slot is logged into ChatGPT or can use Pro.

## Manual ChatGPT Login

Each slot has an independent Chrome profile. In practice, copying an existing
host profile or seed profile may still leave ChatGPT logged out because browser
cookies/session state can be profile, device, version, or provider bound. Treat
per-slot manual login as the reliable setup path.

The stack intentionally publishes no browser or CDP ports. For login, open a
temporary, operator-controlled CDP bridge for one slot at a time, use an SSH
tunnel, finish login, then stop the bridge. Do not expose these ports publicly.

On the server, choose the slot and create a temporary in-container bridge:

```bash
slot=01
container="gpt-webai-slot-$slot"
slot_name="slot-$slot"
cdp_port="$((9222 + 10#$slot))"
bridge_port="$((19000 + 10#$slot))"

docker exec -d \
  --env BRIDGE_PORT="$bridge_port" \
  --env CDP_PORT="$cdp_port" \
  "$container" sh -lc "
  mkdir -p /state/$slot_name/run
  node -e '
    const net = require(\"node:net\");
    const listen = Number(process.env.BRIDGE_PORT);
    const target = Number(process.env.CDP_PORT);
    const server = net.createServer((client) => {
      const upstream = net.connect(target, \"127.0.0.1\");
      client.pipe(upstream);
      upstream.pipe(client);
      const close = () => { client.destroy(); upstream.destroy(); };
      client.on(\"error\", close);
      upstream.on(\"error\", close);
    });
    server.listen(listen, \"0.0.0.0\");
    setInterval(() => {}, 60000);
  ' >/state/$slot_name/run/login-cdp-bridge.log 2>&1 &
  echo \$! >/state/$slot_name/run/login-cdp-bridge.pid
"

container_ip="$(docker inspect -f '{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$container")"
printf 'slot=%s container_ip=%s bridge_port=%s\n' "$slot_name" "$container_ip" "$bridge_port"
```

From your workstation, create an SSH tunnel to the server. Replace `SERVER` with
the host you normally SSH into:

```bash
ssh -N -L 19001:CONTAINER_IP:19001 SERVER
```

Open this URL locally:

```text
http://127.0.0.1:19001/json/list
```

Open the listed `devtoolsFrontendUrl` relative to
`http://127.0.0.1:19001`, use the DevTools screencast to interact with the
ChatGPT page, and complete login for that slot. Repeat for slots `01` through
`10`, changing the local/bridge port to `19002`, `19003`, and so on.

After each slot login, stop the temporary bridge:

```bash
slot=01
container="gpt-webai-slot-$slot"
slot_name="slot-$slot"

docker exec "$container" sh -lc '
  pid_file="/state/'"$slot_name"'/run/login-cdp-bridge.pid"
  if [ -s "$pid_file" ]; then
    kill "$(cat "$pid_file")" 2>/dev/null || true
    rm -f "$pid_file"
  fi
'
```

## Login Verification

Verify every slot through the lifecycle supervisor, not through Docker health:

```bash
for i in $(seq -w 1 10); do
  printf 'slot-%s ' "$i"
  gpt-webai-lifecycle preflight --json --docker-slot-provider --slot "slot-$i" --run-id "login-check-slot-$i"
done

gpt-webai-lifecycle status
```

Expected healthy state:

```text
slot_01_status=ready
...
slot_10_status=ready
```

If a slot shows `auth.needs_login` or `reseed_login`, it is not an authenticated
Pro slot. Do not trust responses, attachments, or ChatGPT sidebar history from
that slot until login verification passes.

## Attachment Handling

`gpt-webai-lifecycle` never passes original host paths directly into the
container. It creates a host-only attachment capsule and exposes generated
read-only filenames to the chosen slot:

```text
/broker-attachments/<request>/<run>/files/NNN-<sha256-16><safeExt>
```

Prompt bytes are staged separately and exposed read-only at
`/broker-prompts/<runId>/prompt.txt`; neither original prompt text nor original
host attachment paths are written to diagnostics, events, or status output.

Directory evidence should be zipped or tarred before attaching:

```bash
python3 -m zipfile -c /path/to/context.zip /path/to/context-dir
gptpro --file /path/to/context.zip "review this evidence"
```

For attachment requests, the lifecycle prompt includes an
`ATTACHMENT_ACCESS_GATE`. If the provider replies `ATTACHMENT_MISSING` or the
wrapper returns `reason:"provider.attachment_unavailable"`, do not treat the
result as a file-based review success.

## Artifact Downloads

When the user explicitly requests a file artifact or Pro claims one as a
deliverable, the provider looks inside assistant turns for
visible controls such as `button.behavior-btn`, filename links, markdown/link
hybrids, and download-adjacent file controls. It arms the Playwright/CDP
download listener before clicking each candidate and saves the result at the
only canonical state-root-relative path:

```text
$STATE/artifacts/<requestKey>/<artifactClaimId>/<artifactId>.download
```

Do not treat an href or presigned URL as final proof. The durable evidence is
the clicked visible element, assistant turn identity, saved host path, size,
SHA-256, MIME/type, `sessionId`, and non-root `https://chatgpt.com/c/...` URL.
If Playwright's CDP download event loses its temp artifact path, the provider may
recover only from a verified browser download directory and marks that metadata
as `recoveredFrom="browser.downloadPath"`.

## GPT Pro Design/Review Workflow

For this Playwright-provider work, GPT Pro owns design and independent design
and implementation review. Codex owns implementation, local verification,
runtime QA, evidence, and the intended commit/push. Before substantive work,
Codex sends a safe current source/evidence bundle to a design Pro, then sends
that design to a separate Pro review session. If the reviewer returns
`CHANGES_REQUIRED`, a fresh designer Pro returns decision-complete
`DESIGN_DELTA_V1` for the failed rows. Codex validates its anchors, hashes,
complete replacements, cross-document closure, and expected digest, applies it
to `.omo/plans/pr72-canonical-design/`, regenerates the manifest, and sends the
complete canonical snapshot to a separate Pro for review. Repeat until all nine
rows PASS and one exact `VERDICT: LGTM_NO_BLOCKING`. Intermediate design
revisions do not require ZIP creation or download. Package, download through a
real Playwright event, and byte-verify the approved final design ZIP once.
Implementation starts only after that design gate passes.

PR #72 remains open and in draft throughout this workflow. The intended
commit/push does not authorize a merge, and Codex must never merge the PR
directly.

Codex implements the approved design directly. It runs the relevant Rust and
Node diagnostics, format/check/lint/tests, repository checks, smoke, and live
browser/service QA. A separate Pro then reviews the complete current source,
diff, and verification evidence. `CHANGES_REQUIRED` sends the work back to
Codex for a root-cause fix and full related re-verification; a design or
high-impact contract change returns to the design gate. Repeat until
one exact `VERDICT: LGTM_NO_BLOCKING` over unchanged final source/diff. Ten
implementation LGTM reviews are not required.

Pro coding source-tree artifacts and no-edit imports are no longer required.
Historical Pro artifacts remain provenance evidence only. Pro verdicts,
filenames, checksum text, and explanations are still untrusted until compared
with current source, tests, screenshots/DOM/CDP, runtime state, and live QA.
On unchanged final source, run the complete live matrix to three consecutive
PASSes. Run timing/race/restart/cooldown/fairness/unknown-session/release-
recovery cases ten times each when repetition is an actual defect-detection
oracle; do not impose arbitrary tenfold repetition on deterministic/static/
schema cases. Any source/test/runtime/contract change resets affected evidence.

The active PR #72 design/review scope is limited to the functional lifecycle
rows below. Retired R10C12/R10C13 specialist prompts, closures, and artifacts
stay local as provenance and are not attached to new Pro requests. New
design/review rows cover functional lifecycle behavior only:
UI/model selection, attachments and send confirmation, session recovery,
artifact downloads, cohort allocation, durable state, CLI/provider envelopes,
release/runtime ownership, and Git/docs/QA scope. Baseline secret handling,
access-control, archive safety, test integrity, destructive-action avoidance,
and PR no-merge rules remain mandatory.

## Recovery Semantics

- A `sessionId` maps to the original `slotId`; resumes stay on that slot.
- New sends rotate across the configured 3/4/3 physical account cohorts and
  within each cohort; pinned `resume`/`show` never switch slot or cohort
  snapshot. Legacy numeric deployment groups do not drive account fairness.
- If all slots are busy or repairing, the supervisor returns a pool-busy/queued
  JSON envelope. Do not duplicate-send the same request; wait for a slot to
  become allocatable, then recover by session key if a session already exists.
- If a slot hits provider/rate limits, it is degraded with `provider.limit` and
  skipped for fresh allocation. The current request avoids that cohort, tries
  each other healthy cohort deterministically, then uses the approved bounded
  provider-limit cooldown budget if all cohorts fail. Cooldown reopen only makes persisted limited slots
  eligible for a fresh provider check; it is not a success signal. Do not repair
  this as a browser failure or count it as a provider answer.
- Persisted `provider.limit` records are not permanent tombstones: stopped
  limited slots become retry candidates after the recorded 3-minute TTL, but
  the next allocation must still capture fresh screenshot/DOM/provider evidence
  and will write `provider.limit` again if ChatGPT is still limited.
- If browser readiness fails before send, or a send attempt returns without a
  `sessionId`, the lifecycle treats the request as not proven to have reached
  ChatGPT and retries with bounded backoff before returning a recovery envelope.
- If send returns a `sessionId` but the provider cannot confirm the matching
  real conversation URL and both server-assigned user/assistant turn IDs, the
  lifecycle records `send.turn_not_proven` through the reconcile/uncertain
  branch, stores start-confirmation evidence, releases/stops the slot runtime,
  and does not count the worker as successful QA. Root URL, counts, generated
  `WEB:` placeholders, content hashes, and Stop controls never satisfy this
  proof.
- If a slot is `repairing`, `warming`, `reseed_login`, or `degraded`, the broker
  will not allocate it for new work.
- Login state is part of readiness. Composer visibility alone is not enough.
- Completion is not only "got an answer". Confirmed terminal answers are written
  to `<fingerprint>.answer.json` and `<fingerprint>.answer.md` before printing
  the final provider response, so compaction or caller interruption can be
  recovered with `gpt-webai-lifecycle show --json --session SESSION_ID
  --fencing-token INVOCATION_TOKEN --docker-slot-provider`.
- Malformed provider resume/poll output and unconfirmed timeout-recovery snippets
  are preserved separately under `<fingerprint>.provider-raw/`; `show` prefers
  that raw artifact for non-`done` records instead of replaying stale display
  transcript text as if it were the latest answer.
- `response-recovered-after-timeout` plus `responseStableMs=0` and in-progress
  answer language is `answer.recovery_incomplete` / `answer_unconfirmed`, not a
  terminal completion. A recovered answer is still valid when it is terminal and
  not merely a planning/progress sentence.
- A supervised use is operationally complete only after the wrapper/lifecycle
  process exits and `gpt-webai-lifecycle status` shows `holders=0`, `locks=0`,
  the lifecycle-owned runtime stopped, and the used slot `standby`, `exited`,
  `allocatable`, or in an explicit blocked state. A stopped runtime must not be
  reported as `ready`; the next use starts the container/browser again.
- Do not leave a manual/debug slot session half-open. After any interrupted
  wrapper, SSH disconnect, manual browser probe, or operator CDP bridge, first
  recover the answer with `gpt-webai-lifecycle show --json --session SESSION_ID
  --fencing-token INVOCATION_TOKEN --docker-slot-provider` if a `sessionId`
  exists, then run `gpt-webai-lifecycle release --json --session SESSION_ID
  --fencing-token INVOCATION_TOKEN` or `gpt-webai-lifecycle release --json
  --slot SLOT_ID --fencing-token INVOCATION_TOKEN`. Re-check `status --json`;
  if stale holders or locks remain, run `gpt-webai-lifecycle cleanup --json
  --apply` and re-check.
  Tokenless manual release refuses active leases and only releases stale or
  already-missing locks; active work must be allowed to finish or be diagnosed
  before release.

## Security Notes

- No CDP ports are published in compose.
- Temporary login bridges are operator-only and must be stopped after login.
- Do not commit, print, or attach Chrome profile files, cookies, tokens, or
  `$STATE/auth-seed/**`.
- Do not delete user Chrome profiles during cleanup.
- Slot attachment mounts are read-only and contain generated filenames, not
  original host paths.

## Smoke Checks

```bash
bash -n stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle
bash -n stacks/gpt-webai-slot-pool/scripts/slot-entrypoint.sh
bash -n stacks/gpt-webai-slot-pool/scripts/slot-healthcheck.sh
```

Run every aggregate R13 gate from the stack directory. The four modes are
separate acceptance surfaces; none substitutes for another.

```bash
cd stacks/gpt-webai-slot-pool
bash tests/gpt-webai-lifecycle/test.sh static
bash tests/gpt-webai-lifecycle/test.sh fake
bash tests/gpt-webai-lifecycle/test.sh full
GPT_WEBAI_LIVE=1 bash tests/gpt-webai-lifecycle/test.sh smoke
```

Live QA is provider-facing and creates real ChatGPT conversations. Complete
login readiness first, compute the content-only source fingerprint, and run the
complete L01-L21 matrix three consecutive times on unchanged source bytes:

```bash
cd stacks/gpt-webai-slot-pool
SOURCE_FINGERPRINT="$(bash scripts/qa-fingerprint-r13.sh --print)"
bash scripts/qa-live-matrix-r13.sh --iteration 1 --source-fingerprint "$SOURCE_FINGERPRINT"
bash scripts/qa-live-matrix-r13.sh --iteration 2 --source-fingerprint "$SOURCE_FINGERPRINT"
bash scripts/qa-live-matrix-r13.sh --iteration 3 --source-fingerprint "$SOURCE_FINGERPRINT"
```

Only R01-R10 are repeated ten times because their live oracle depends on a
timing, race, restart, cooldown, fairness, unknown-session, or release-recovery
condition. Deterministic/static/schema cases are not given an arbitrary
ten-repeat gate.

```bash
for case_id in R01 R02 R03 R04 R05 R06 R07 R08 R09 R10; do
  for repetition in $(seq 1 10); do
    bash scripts/qa-live-matrix-r13.sh \
      --targeted-case "$case_id" \
      --repetition "$repetition" \
      --source-fingerprint "$SOURCE_FINGERPRINT"
  done
done
```

For the live smoke matrix, see
`stacks/gpt-webai-slot-pool/SMOKE_TESTS.md`.
