# gpt-webai-lifecycle Runbook

Use this only when `gptpro`, `gptxhigh`, or `gpt-webai-lifecycle` returns a browser/CDP/slot recovery envelope. Do not load it for normal GPT delegation.

## Entrypoints

- Pro Extended: `gptpro "prompt"` or `printf '%s\n' "prompt" | gptpro`
- Thinking/xhigh: `gptxhigh "prompt"` or `printf '%s\n' "prompt" | gptxhigh`
- File attachments: `gptpro --file /path/to/context.zip "prompt"`; repeat
  `--file` for multiple files. Direct lifecycle calls use the same flag:
  `gpt-webai-lifecycle run --kind pro --file /path/to/context.zip --prompt "prompt"`.
  Directories should be zipped or tarred first.
- Resume existing session:
  - `gpt-webai-lifecycle resume --kind pro --session "<SESSION_ID>"`
  - `gpt-webai-lifecycle resume --kind xhigh --session "<SESSION_ID>"`
- Status/cleanup:
  - `gpt-webai-lifecycle status`
  - `gpt-webai-lifecycle cleanup --dry-run`
  - `gpt-webai-lifecycle cleanup --apply`
- Queued work:
  - `gpt-webai-lifecycle queue resume --request "<FINGERPRINT>"`
- Help/usage:
  - `gpt-webai-lifecycle --help`
  - Do not use `gptpro --help` or `gptxhigh --help`: those wrappers treat
    non-`--file` arguments as prompt text and can create a real ChatGPT session.

## Session Semantics

- `gptpro` and `gptxhigh` create a new delegated prompt unless the exact same
  request fingerprint already has a recorded session.
- `gpt-webai-lifecycle resume --kind ... --session SID` only resumes, polls, or
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
- A session record with `slotId` is pinned to that original slot. If the slot
  pool is unavailable, follow the recovery envelope and restore the slot pool
  before resuming. A record without `slotId` is unsupported by the slot-only
  lifecycle and must not fall back to a host-local browser/CDP path.
- Slot repair is bounded by per-action attempts and `nextRetryAt` backoff.
  It also respects the slot runtime lease. Do not allocate or repair a busy,
  repairing, warming, reseed_login, or degraded slot. If a browser/slot
  recovery envelope names `browser ensure --slot slot-XX`, run that
  slot-specific command.
- Slot readiness includes ChatGPT auth state, not only composer visibility. If
  the page shows login/signup or anonymous-use copy, the slot is
  `auth.needs_login`/`reseed_login`; do not treat responses, attachments, or
  sidebar history from that slot as authenticated Pro work.
- A send attempt without `sessionId` is not proof that ChatGPT received the
  prompt. Lifecycle retries this pre-session failure with bounded backoff
  before returning `send.unknown_session`. Treat `send.unknown_session` as "not
  complete"; do not count it as a provider answer or attachment verdict.
- `browser ensure` without `--slot` is not a GPT delegation recovery path in
  slot-only mode.

## Slot Release Contract

- Every GPT slot use must enter and leave through `gptpro`, `gptxhigh`, or
  `gpt-webai-lifecycle`. Do not raw-use `agbrowse web-ai` or leave a manual
  Chrome/CDP session as the operational path.
- A supervised use is not finished when the answer text is visible. It is
  finished only after the wrapper/lifecycle command exits and the slot runtime
  lease is released.
- After any interruption, aborted terminal, SSH disconnect, manual CDP bridge,
  or operator browser probe, run:
  ```bash
  gpt-webai-lifecycle status
  ```
  If it reports stale holders or stale locks, run:
  ```bash
  gpt-webai-lifecycle cleanup --apply
  gpt-webai-lifecycle status
  ```
- The required release evidence is `holders=0`, `locks=0`, and the affected
  slot status back to `ready`. Do not start a new manual slot action until this
  is true.
- Current lifecycle release writes the slot `ready` state and releases its
  runtime lock. It does not stop Chromium or the slot container. Browser
  shutdown-on-release is a separate implementation requirement; do not document
  or assume per-use browser clean-start behavior until that code exists.

## Attachments

- `--file` accepts regular readable files only. Zip/tar directories before
  attaching.
- In Docker slot mode, original host paths are never passed to container
  `agbrowse`.
- Lifecycle creates a host-only attachment capsule, then exposes generated
  filenames only to the selected slot under read-only
  `/broker-attachments/.../NNN-<sha256-16><safeExt>`.
- Attachment capsule/mount visibility is not the same as provider/model
  readability. For requests with attachments, lifecycle adds an
  `ATTACHMENT_ACCESS_GATE` using generated filenames, sizes, and hashes. If the
  model replies `ATTACHMENT_MISSING`, lifecycle returns
  `ok:true,status:"recovering",reason:"provider.attachment_unavailable"` and the
  result must not be treated as a successful attachment-based review.
- The original user file is never deleted or mutated by lifecycle cleanup.
- Logs/events/status/cleanup must not contain prompt text, raw provider output,
  cookies, tokens, browser state files, attachment contents, original attachment
  paths, or original attachment filenames.
- Auth seed state is not an attachment. Never mount, attach, log, or
  cleanup-delete `$STATE/auth-seed/**`.

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
5. If `$DISPLAY` is missing, start or install Xvfb. Do not use `CHROME_HEADLESS=1`; web-ai needs headed Chrome.
6. Re-check with `gpt-webai-lifecycle status`, then follow only the returned `resumeCommand` or `nextCommand`.

## Recovery Rules

- `status` and `cleanup --dry-run` are enough as evidence. Do not loop on manual probes.
- If a wrapper process is still running, keep polling that exec session.
- If a free slot exists, a new request may start even while another slot is busy.
- Never kill Chrome, delete slot browser state, prune sessions, or call raw
  `agbrowse web-ai` outside supervisor/slot cleanup. If a future lifecycle
  release command owns browser shutdown, use that command instead of ad hoc
  process kills.
