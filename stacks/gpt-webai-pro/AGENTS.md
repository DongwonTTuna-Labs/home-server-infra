# gpt-webai-pro — agent contract

Repository-local rules for work on `stacks/gpt-webai-pro`. They add to the
global agent rules; they do not replace them. This contract used to live in the
global `AGENTS.md`, where it loaded in every unrelated session.

The authority for this stack is its current `DESIGN.md`, `README.md`, source,
tests, generated config, runtime state, and live evidence — in that order over
any summary, including this file.

## Ownership

Codex owns implementation, diagnosis, verification, and operational rollout
here. Subagents and external models review and cross-check; they do not own the
edit.

Diagnose an anomaly before editing, and fix the layer that actually caused it —
implementation, contract, or environment. Keep behavior, design, tests,
scripts, and operational docs consistent inside one focused change. Do not
revive a retired implementation to fill a gap.

Use risk-based verification: static checks, deterministic tests, real-browser
or container smoke, and live service checks as they apply. Repeat a check only
when concurrency, timing, restart, cooldown, or fairness makes repetition a
useful detector.

## Runtime Contract

The single entry point is `gpt-webai-pro` (`run`, `resume`, `status`,
`cleanup`, `release`, `login`, `keepalive`, `image-batch`, `inspect`). The daemon runs on the home
server; the Mac reaches it over `ssh home`.

- `run` does not attach a delegation prelude. The remote model knows nothing
  about local files, PRs, logs, or tool state. Attach what it needs with
  `--file PATH` (repeatable; zip or tar a directory) and put the required
  context in the prompt body instead of summarizing it away.
- A delegation prompt carries objective, verdict format, constraints, an
  attachment manifest, the exact questions, and known omissions. Tell the
  reviewer to fail with a list of missing evidence rather than guess. State
  that attachments outrank any summary.
- Output is a JSON envelope: `{ok, hardFailure, networkDisconnected,
  usageError, status, sessionId, resumeCommand, answer, answerPath, artifacts,
  errorKind, message}`.
- `hardFailure:true` with `networkDisconnected:true` (exit 1) is only for
  direct evidence that chatgpt.com is unreachable. Lifecycle, input, auth,
  browser, and pool problems are exit 0 envelopes with `status` of `failed`,
  `needs_user_action`, `recovering`, or `running`. An empty prompt is exit 0,
  `usageError:true`, `status:"needs_user_action"`, and never touches a browser.
- The default timeout is `GPTPRO_TIMEOUT` seconds (10800). A timeout is not a
  failure: it returns `status:"running"` plus `resumeCommand`. Continue that
  `sessionId`; never resend as a new prompt. Duplicate-send protection is
  `flock` plus the `send_attempts` row, not your judgement.
- `status:"uncertain"` / `errorKind:"send_uncertain"` is resolved only by
  `gpt-webai-pro resume --session req_...`, which reconciles: it recovers a
  landed turn or fails closed. `recovering` / `pool_busy` means every slot is
  busy — retry with `resumeCommand`.
- A slot in `needs_login` is a human's job. Report it; recovery is
  `gpt-webai-pro login --slot slot-a|slot-b|slot-c`, which brings up noVNC on
  loopback for a person to log in. `keepalive` (systemd user timer, 09:20)
  maintains sessions.
- ChatGPT-rendered file artifacts are never reconstructed from answer text or a
  URL. Truth is what the supervisor captured through the download event into
  `requests/<id>/artifacts/` with a recorded sha256 and size — the envelope's
  `artifacts[]`.

## Do Not

- Do not delete, stop, or `docker exec` into the state directory
  (`${GPT_WEBAI_PRO_STATE_DIR:-$HOME/.local/state/gpt-webai-pro}`), a slot
  profile, or a running `gwp-slot-*` container. Cleanup and recovery go through
  `gpt-webai-pro cleanup` and `release`.
- Do not use the MCP `web_ai_*` tools, raw agbrowse or Playwright scripts, or a
  direct provider call as a delegation path.
- Do not reintroduce v1 (`gpt-webai-lifecycle`, slot-pool, `gptxhigh`), which
  was retired in 2026-07 and whose binaries are gone, or the `resume --kind`,
  `show --session`, `--slot slot-NN`, cohort, broker-attachments, and auth-seed
  concepts. The observed September 2026 UI selects Pro through a Power control
  and the model through a separate Latest radio; older Intelligence radios are
  still supported. A hidden slider may expose state while its visible Power
  menuitem receives keyboard input. Source, tests, and observed DOM outrank old
  UI descriptions.
  전용 `image-batch`는 Xhigh 경로로, 구 UI의 Extra High / 새 UI의 Pro 바로 아래
  Extended를 사용한다. 일반 위임은 Pro다.
- The retired PR #72 contracts — canonical-delta, Manager-Only, artifact-only,
  no-edit/import, quarantine, fixed review count — are historical provenance
  only and do not govern current work.

## Operating Procedure

`stacks/gpt-webai-pro/README.md` is the runbook, with the client-side view in
`~/.codex/runbooks/gpt-webai-pro.md`. The former `gpt-webai-lifecycle.md` and
`gpt-slot-login-console.md` runbooks were removed; do not restore them.
