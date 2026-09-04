# gptpro-review Runbook

Use this runbook for `/home/dongwonttuna/Documents/Programming/gptpro-review`.
The only supported stack is Rust lifecycle/server with Postgres, Bun with the
official Playwright API, and the Svelte dashboard. The only model command is
`pro`.

## First use

```sh
cd /home/dongwonttuna/Documents/Programming/gptpro-review
bun run setup
bun run local:status
curl --fail http://127.0.0.1:8787/health/ready
curl --fail http://127.0.0.1:8787/health/provider
```

`setup` owns all fixed local values and prepares the repository-pinned Bun,
locked dependencies, Playwright Chromium, Postgres, Rust API, Svelte dashboard,
and the repository-owned persistent browser profile. Do not provide a runtime
archive, CDP endpoint, slot, API address, port, or environment variable. The
versioned `vendor/bun-linux-x64.zip` must match the installer SHA-256 and pinned
revision; a missing or mismatched archive fails instead of using a mutable
download or a different host Bun.

ChatGPT credentials are the single human authority boundary. Enroll the
repository-owned profile once:

```sh
bun run login
```

Open the tokenized loopback URL printed by the command, complete login in the
real browser view, and select `로그인 완료 확인`. The console supports screen
clicks, Shift-modified text, and paste without logging the input. It never
copies cookies or adopts another Chrome/profile. Then verify the actual picker:

```sh
bun run pro --operation verify
```

Only a visible `Pro` option selected and reverified from screenshot plus
sanitized DOM/CDP evidence can publish `ready`. Logged-out, unavailable, or
unverified UI is not ready and is never downgraded or skipped.

## Verification

```sh
bun run check
bun run test
bun run build
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun run smoke
```

`bun run smoke` is always a live-browser smoke, not an opt-in mode. It verifies
the disposable Postgres recovery path, API, CORS, dashboard, real Pro picker,
a unique canary send, pinned poll/resume until a non-empty assistant response,
the exact response SHA-256, absence of a download control for the text-only
answer, and clean release. Login missing, Pro missing, DOM drift, response
mismatch, or cleanup residue fails the run.

The run writes `.evidence/smoke/<timestamp>/`. Require `VERIFY_OK`; inspect
`summary.log`, every recorded return code, managed-browser screenshots,
sanitized DOM/CDP, receipts, final session state, and cleanup port files before
reporting success. Never infer browser state from containers or another task.

## Recovery

```sh
bun run local:status
bun run local:down
bun run setup
```

`local:status` and `local:down` intentionally do not execute Bun, so they still
work if the pinned runtime is missing. They inspect or stop only the documented
owned user units and Postgres container, preserve the database volume and
browser profile, and refuse unowned resources. Do not replace them with broad
process kills, container deletion, raw agbrowse, or arbitrary CDP attachment.
