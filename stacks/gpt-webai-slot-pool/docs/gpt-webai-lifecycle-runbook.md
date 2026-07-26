# gpt-webai-lifecycle Runbook

For PR #72, the active design/review and live-QA scope is limited to the current
functional lifecycle rows. Retired R10C12/R10C13 specialist materials are local
provenance only and are not attached to new Pro requests. Baseline secret
handling, access-control, archive safety, test integrity, destructive-action
avoidance, and no-merge invariants remain mandatory.

Use this only when `gptpro`, `gptxhigh`, or `gpt-webai-lifecycle` returns a browser/CDP/slot recovery envelope. Do not load it for normal GPT delegation.

## Entrypoints

- Pro/standard: `gptpro "prompt"` or `printf '%s\n' "prompt" | gptpro`
- Thinking/xhigh: `gptxhigh "prompt"` or `printf '%s\n' "prompt" | gptxhigh`
- File attachments: `gptpro --file /path/to/context.zip "prompt"`; repeat
  `--file` for multiple files. Direct lifecycle calls use the same flag:
  `gpt-webai-lifecycle run --kind pro --file /path/to/context.zip --prompt "prompt"`.
  Directories should be zipped or tarred first.
- Resume existing session:
  - `gpt-webai-lifecycle resume --json --session "<SESSION_ID>" --fencing-token "<INVOCATION_TOKEN>" --docker-slot-provider`
- Status/cleanup:
  - `gpt-webai-lifecycle status --json`
  - `gpt-webai-lifecycle cleanup --json --dry-run`
  - `gpt-webai-lifecycle cleanup --json --apply`
- Visual readiness before a live wait/send:
  - `gpt-webai-lifecycle preflight --json --docker-slot-provider --slot slot-XX --run-id "<RUN_ID>"`
- Help/usage:
  - `gpt-webai-lifecycle --help`
  - Do not use `gptpro --help` or `gptxhigh --help`: those wrappers treat
    non-`--file` arguments as prompt text and can create a real ChatGPT session.

## Session Semantics

- `gptpro` and `gptxhigh` create a new delegated prompt unless the exact same
  request fingerprint already has a recorded session.
- `gpt-webai-lifecycle resume --json --session SID --fencing-token TOKEN
  --docker-slot-provider` only resumes, polls, or
  recovers an existing session result. It does not append a new user message,
  send revised files, or preserve a review loop as a same-conversation follow-up.
- After a send returns a `sessionId`, do not resend the same prompt because of
  timeout, CDP, poll, or capture issues. Resume the same `sessionId`.
- If a revised artifact needs another review and no explicit supported tool/UI
  path exists for appending to the old conversation, send a new `gptpro` or
  `gptxhigh` request. Include the prior verdict/session context in the prompt
  and report the result as a new-session re-review.
- In slot mode, an active wrapper/resume process or busy slot does not globally
  block new work. Never duplicate-send the same session/fingerprint, but a free
  GPT slot may accept a new request. If all slots are busy, warming, repairing,
  reseed_login, or degraded, follow the queued/recovering envelope.
- Fresh sends rotate across the configured physical account cohorts. The
  current preserved layout is `slot-01..03=cohort-a`,
  `slot-04..07=cohort-b`, and `slot-08..10=cohort-c`; a pinned resume/show
  stays on the original recorded slot and cohort snapshot. Fresh allocation
  also rotates within each cohort using persisted cursors. Legacy
  `group-01/group-02` labels may describe deployment topology only and are not
  account fairness, provider-limit, cooldown, or authorization authority.
- A session record with `slotId` is pinned to that original slot. If the slot
  pool is unavailable, follow the recovery envelope and restore the slot pool
  before resuming. A record without `slotId` is unsupported by the slot-only
  lifecycle and must not fall back to a host-local browser/CDP path.
- Slot repair is bounded by per-action attempts and runtime leases. Do not
  allocate or repair a busy, repairing, warming, reseed_login, or degraded
  slot. Use `preflight --json --docker-slot-provider --slot slot-XX --run-id
  RUN_ID` for read-only screenshot/DOM readiness evidence before any live send
  or long wait.
- Slot readiness includes ChatGPT auth state, not only composer visibility. If
  the page shows login/signup or anonymous-use copy, the slot is
  `auth.needs_login`/`reseed_login`; do not treat responses, attachments, or
  sidebar history from that slot as authenticated Pro work.
- Slot readiness also includes ChatGPT Pro/model evidence. An initial
  `Instant`, `Extra High`, or other model label on a Pro request is not an
  immediate mismatch. Capture actual Chrome screenshot+sanitized DOM/CDP,
  open the picker, select visible/available `Pro`, then recapture and verify
  composer label `Pro` before upload/send. Boundedly re-discover stale/drifted
  picker controls and scroll options as needed. Emit
  `model.selection_mismatch`/`auth.needs_pro` only after picker-proven absence
  or bounded selection/reverification failure, then rotate to another healthy
  cohort or return an evidenced `recovering`/`queued` envelope. Silent
  downgrade is forbidden.
- Provider-level R13 picker absence reasons are `picker.model_absent` and
  `picker.effort_absent`; bounded failures use `picker.control_drift`,
  `picker.selection_timeout`, or `picker.reverify_mismatch`.
- Provider/rate-limit UI such as "too many requests", message cap, request
  limit, or try-again-later text is also send-blocking. Treat it as
  `provider.limit`/degraded evidence for that slot/account group; do not repair
  it as a browser failure or count it as a provider answer.
- For provider limits, do not rely on dismissing a modal such as `Got it`.
  Dismissal is not recovery unless a new screenshot/DOM gate and a real canary
  send prove the composer can send again. The supervised fallback is: try each
  distinct healthy cohort in deterministic order; if every cohort reports
  `provider.limit`, use the approved bounded cooldown and retry the full cohort
  sequence. A cooldown can reopen persisted `provider.limit` slots only
  for a fresh provider check; screenshot/DOM/send evidence must still prove
  recovery.
- Persisted `provider.limit` slot state records `provider_limit_observed_at_ms`
  and `provider_limit_next_retry_at_ms`. Stopped limited slots become standby
  candidates after the 3-minute recheck TTL so the next request can refresh the
  provider state. This is not a background refresh loop and not proof of
  recovery; if the next visual/provider gate still sees provider limit, the slot
  remains `provider.limit` with a fresh retry timestamp.
- A send attempt without `sessionId` is not proof that ChatGPT received the
  prompt. Lifecycle retries this pre-session failure with bounded backoff
  before returning `send.unknown_session`. Treat `send.unknown_session` as "not
  complete"; do not count it as a provider answer or attachment verdict.
- A send attempt with `sessionId` is still not confirmed until lifecycle sees
  the matching non-root `https://chatgpt.com/c/...` conversation URL and both
  new server-assigned `data-message-id` identities (user and assistant). If
  either identity or the URL is absent/mismatched, reconciliation is read-only;
  an unproven turn becomes `send.turn_not_proven`. Turn counts, text, DOM index,
  timestamps, Stop controls, and `WEB:` placeholders are never fallbacks.
- Start confirmation waits for the URL and both message identities together
  because ChatGPT can show active generation before the `/c/...` URL or
  assistant node is visible. This is only delay tolerance; root URL plus active
  generation is still unconfirmed at the timeout boundary.
- A root ChatGPT composer can be valid pre-send readiness evidence only when
  screenshot/DOM shows authenticated Pro composer readiness. It is never send
  success evidence.

## Slot Release Contract

- Every GPT slot use must enter and leave through `gptpro`, `gptxhigh`, or
  `gpt-webai-lifecycle`. Do not raw-use browser-agent commands, MCP `web_ai_*`,
  or ad hoc Playwright scripts as the operational path.
- A supervised use is not finished when the answer text is visible. On terminal
  `done`, lifecycle first stores `<fingerprint>.answer.json` and
  `<fingerprint>.answer.md`, then prints the provider response, then releases the
  slot.
- If Codex compaction, terminal interruption, or output loss happens after a
  `sessionId` exists, do not resend first. Recover with:
  ```bash
  gpt-webai-lifecycle show --json --session SESSION_ID \
    --fencing-token INVOCATION_TOKEN --docker-slot-provider
  ```
- For PR #72 work, Pro owns design and independent design/implementation
  review; Codex owns implementation, local verification, live QA, evidence, and
  intended commit/push. Transcript and verdict text are evidence, not truth.
  Codex compares every Pro claim with current source/diff, tests,
  screenshots/DOM/CDP, runtime status, and live behavior.
- Before substantive Playwright-provider changes, send a safe current
  source/evidence/design bundle to a design Pro. Send the resulting design to a
  separate Pro reviewer. If it returns `CHANGES_REQUIRED`, have the design Pro
  revise it and repeat the independent review until `LGTM/no-blocking`.
  Codex then implements the approved design directly and runs all relevant
  Rust/Node/repository/smoke/live checks.
- Send the completed source/diff and verification evidence to a separate Pro
  implementation reviewer. `CHANGES_REQUIRED` returns to Codex for a
  root-cause fix and full related re-verification; a design or high-impact
  contract change returns to the design gate. Repeat until
  `LGTM/no-blocking`. Pro coding source-tree artifacts and Codex no-edit imports
  are no longer required; historical artifacts remain provenance evidence.
  Final source stability requires one fresh independent implementation review
  over the unchanged final source/diff/evidence with exact
  `VERDICT: LGTM_NO_BLOCKING`. Source/test/runtime/contract changes invalidate
  that verdict and require a new review; ten repeated reviews are not a gate.
- Release is mandatory. Normal `run`, `resume`, and artifact `download` perform
  it in the lifecycle path. After any aborted terminal, SSH disconnect, manual CDP
  bridge, or operator browser probe, run:
  ```bash
  gpt-webai-lifecycle release --json --session SESSION_ID --fencing-token INVOCATION_TOKEN
  # or, if no sessionId was recorded:
  gpt-webai-lifecycle release --json --slot slot-XX --fencing-token INVOCATION_TOKEN
  gpt-webai-lifecycle status --json
  ```
  Tokenless manual release refuses active leases (`reason=lock.active`) and
  only releases stale or already-missing locks. If the lease is active, do not
  force it; inspect the live process/session first.
  If status reports stale holders or stale locks, run:
  ```bash
  gpt-webai-lifecycle cleanup --json --apply
  gpt-webai-lifecycle status --json
  ```
- The required release evidence is `holders=0`, `locks=0`, and reconciled slot
  state. `ready` means a live runtime/CDP/provider check currently proves an
  authenticated Pro composer. If terminal release stops the runtime, the slot
  should be `standby`/allocatable rather than falsely `ready`; an exited runtime
  must be `exited`, `repairing`, or another explicit non-ready state. The next
  allocation starts the slot container/browser again. Do not manually kill
  Chrome/container processes outside lifecycle release/cleanup.

## Attachments

- `--file` accepts regular readable files only. Zip/tar directories before
  attaching.
- In Docker slot mode, original host paths are never passed to container
  provider commands.
- Lifecycle creates a host-only attachment capsule, then exposes generated
  filenames only to the selected slot under read-only
  `/broker-attachments/.../NNN-<sha256-16><safeExt>`.
- Attachment capsule/mount visibility is not the same as provider/model
  readability. For requests with attachments, lifecycle adds an
  `ATTACHMENT_ACCESS_GATE` using generated filenames, sizes, and hashes. If the
  model replies `ATTACHMENT_MISSING`, lifecycle returns
  `ok:true,status:"recovering",reason:"provider.attachment_unavailable"` and the
  result must not be treated as a successful attachment-based review.
- The provider must also wait for ChatGPT-rendered attachment chip or equivalent
  upload-complete UI for every staged file before send. Missing upload UI is
  `provider.attachment_unavailable`, not a successful attachment result.
- Provider receipts name root capture `capture.root` and stale attachment
  evidence `ChipProof`; retained R5 `capture` and `StaleChipProof` are not
  current terms.
- The original user file is never deleted or mutated by lifecycle cleanup.
- Logs/events/status/cleanup must not contain prompt text, raw provider output,
  cookies, tokens, browser state files, attachment contents, original attachment
  paths, or original attachment filenames.
- Auth seed state is not an attachment. Never mount, attach, log, or
  cleanup-delete `$STATE/auth-seed/**`.

## Download Artifacts

- Production artifact collection is UI-first. The provider looks inside visible
  assistant turns for filename controls such as `button.behavior-btn`, download
  links, `a[download]`, and markdown/UI hybrids.
- The provider arms the Playwright/CDP download listener before clicking each
  candidate and saves the file at the canonical state-root-relative path:
  ```text
  $STATE/artifacts/<requestKey>/<artifactClaimId>/<artifactId>.download
  ```
- The saved `.download` filename makes the receipt media type
  `application/octet-stream`; the provider preserves the suggested filename as
  evidence but never uses it as the host path or MIME oracle.
- In CDP slot mode, Playwright can report a download but fail `saveAs()` if its
  temporary artifact path is missing. Treat that as recoverable only when the
  same suggested filename exists in a verified browser download directory and
  the provider records `recoveredFrom="browser.downloadPath"`.
- A URL or href is debug evidence only. It is not the artifact truth. Required
  proof is the visible element snapshot, turn identity, saved host path, size,
  SHA-256, MIME/type, `sessionId`, and non-root `/c/...` conversation URL.
- `artifacts[]` can be empty only when artifacts are optional and none were
  requested/claimed. If a required/claimed artifact has no visible control or no
  Playwright download event, classify it as `artifact.controls_absent`,
  `artifact.download_timeout`, or `artifact.recovery_failed`; do not rebuild the
  file from answer text.

## Delegation Prompt Policy

`gptpro` and `gptxhigh` automatically prepend
`~/.codex/prompts/gpt-delegation-prelude.md` before the user prompt. This is
intentional: delegated GPT work must return the complete requested outcome, not
the smallest visible substep.

If you must call `gpt-webai-lifecycle run --kind ... --prompt ...` directly
instead of the wrappers, manually prepend that same prelude. Do not send a
delegated Pro/xhigh prompt without the complete-spec prelude.

## Delegation Evidence Bundle

Use this for GPT review, validation, design, or spec tasks. The receiving model
is blind to local files, PRs, logs, MCP/tool state, and service state unless
they are attached or summarized as evidence.

Context ladder:

1. Current request and required verdict/output format.
2. Files directly changed or planned to change.
3. Entrypoints, wrappers, CLIs, and generated config that execute the behavior.
4. Tests, fixtures, smoke output, failing logs, and current status output.
5. Current PR body/state when PR semantics matter.
6. Relevant AGENTS, runbook, spec, security, or API contracts.
7. Tooling surface: MCP/config/wrapper/status summaries when they affect the task.
8. Neighboring callsites, ownership boundaries, migrations, or provider contracts
   for high-risk behavior.

Evidence gate:

- Before sending, ask: can the receiving model decide from attachments without
  local access? If not, attach more evidence or list the exact omission.
- Instruct the reviewer not to speculate. If required evidence is missing, it
  must return `CHANGES_REQUIRED` or the requested failure verdict and name the
  missing evidence needed for a real review.

Prompt skeleton:

```text
Task: <complete desired outcome>
Verdict format: <LGTM|CHANGES_REQUIRED or requested format>
For design or review work, return the requested decision-complete design or
evidence-grounded `LGTM`/`CHANGES_REQUIRED` verdict. Codex owns source/test/
runtime/script implementation and repairs under the approved design. A
downloadable artifact is required only when the user explicitly asks for one or
the Pro claims a file artifact as its deliverable; in that case it must be
verified through the visible control and a real Playwright download event.
Current truth is in attachments. If this summary conflicts with attached files,
trust the attachments.
Do not speculate beyond the attached evidence. If evidence is insufficient,
return the failure verdict and list the exact missing evidence.

Attachment manifest:
- <path>: why it matters
- <path>: why it matters

Constraints:
- <safety/auth/test/PR/tool constraints>

Questions:
- <specific things to validate>

Known omissions:
- <none, or exact missing evidence>
```

Token policy:

- Attach full small files.
- Zip or tar directories and multi-file evidence bundles.
- For huge/generated/log files, attach focused excerpts plus `rg` or file-list
  output.
- Prefer evidence files over long inline paste.
- Do not attach secrets; attach redacted shape, variable names, paths,
  permissions, and handling rules instead.
- Do not require web search for local implementation truth. Ask for web checks
  only for external facts that may have changed, such as Docker/API/cloud or
  provider documentation.

## Chrome/Chromium Recovery

If the wrapper reports `CDP connection failed`, `ECONNREFUSED 127.0.0.1:9222`, `Chrome not found`, `No usable sandbox`, or display errors:

1. Check Chrome/Chromium:
   ```bash
   command -v google-chrome || command -v chromium || command -v chromium-browser || command -v chrome || true
   ```
2. If no system Chrome exists, install Playwright Chromium in user cache:
   ```bash
   npx playwright install chromium
   ```
3. Find the binary and export it:
   ```bash
   for p in "$HOME"/.cache/ms-playwright/chromium-*/chrome-linux/chrome "$HOME"/.cache/ms-playwright/chromium-*/chrome-linux64/chrome; do [ -x "$p" ] && printf '%s\n' "$p"; done
   export CHROME_BINARY_PATH="$HOME/.cache/ms-playwright/chromium-<VERSION>/chrome-linux64/chrome"
   ```
4. If sandbox errors appear, use:
   ```bash
   CHROME_BINARY_PATH="$CHROME_BINARY_PATH" CHROME_NO_SANDBOX=1 gptpro "real prompt, not a smoke prompt"
   ```
5. If `$DISPLAY` is missing, start or install Xvfb. Do not use
   `CHROME_HEADLESS=1`; the ChatGPT provider needs headed Chrome.
6. Re-check with `gpt-webai-lifecycle status`, then recover by persisted
   session key with `show`, `resume`, `download`, or safe `release`.

## Recovery Rules

- `status` and `cleanup --dry-run` are enough as evidence. Do not loop on manual probes.
- If a wrapper process is still running, keep polling that exec session.
- If a free slot exists, a new request may start even while another slot is busy.
- Never kill Chrome, delete slot browser state, prune sessions, or call raw
  provider/browser commands outside supervisor/slot cleanup. Lifecycle release
  owns slot browser shutdown; use `release --session` or `release --slot`
  instead of ad hoc process kills.
