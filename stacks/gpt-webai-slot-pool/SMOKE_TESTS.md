# GPT WebAI Slot Pool R13 Smoke and Live QA

This document describes the R13 functional lifecycle acceptance surface. Live
commands create real ChatGPT conversations and downloads. Do not run them until
the target slots are authenticated and the source under test is stable.

The suite never treats a root `https://chatgpt.com/` URL, a placeholder
`WEB:...` identifier, an empty worker output, or active generation without a
non-root `/c/<sessionId>` binding as send success. A fresh send succeeds only
after the same bounded confirmation window proves the user turn, assistant
start, real conversation URL, pinned slot, and session binding.

## Local aggregate gates

Run all four modes from `stacks/gpt-webai-slot-pool`. They are separate
acceptance surfaces and all four are required.

```bash
bash tests/gpt-webai-lifecycle/test.sh static
bash tests/gpt-webai-lifecycle/test.sh fake
bash tests/gpt-webai-lifecycle/test.sh full
GPT_WEBAI_LIVE=1 bash tests/gpt-webai-lifecycle/test.sh smoke
```

The direct Rust and Node gates remain authoritative sensors as well:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace
cargo test --workspace --all-targets --all-features -- --test-threads=1
npm --prefix provider/chatgpt-playwright ci
npm --prefix provider/chatgpt-playwright test
```

No test may be deleted, skipped, weakened, replaced with generated expected
output from the implementation under test, or substituted with a fake success
for a live case.

## Source fingerprint

The live counter key is the content-only source fingerprint:

```bash
SOURCE_FINGERPRINT="$(bash scripts/qa-fingerprint-r13.sh --print)"
export SOURCE_FINGERPRINT
```

The command prints exactly one lowercase hexadecimal line. Runtime/evidence
outputs, `target/`, `node_modules/`, and mutable Git/PR object IDs are excluded.
Repository source, config, tests, docs, scripts, canonical design bytes,
wrapper/shim identities, and authority-input hashes are included. Any affected
source/test/runtime/provider-contract byte change resets the corresponding
matrix and repetition counters.

## Complete L01-L21 live matrix

Run three consecutive complete iterations on identical source bytes:

```bash
for iteration in 1 2 3; do
  bash scripts/qa-live-matrix-r13.sh \
    --iteration "$iteration" \
    --source-fingerprint "$SOURCE_FINGERPRINT"
done
```

The script runs these cases in this exact order:

1. `L01` preflight initial exact Pro
2. `L02` preflight initial non-Pro then picker-first Pro/standard
3. `L03` fresh text run
4. `L04` fresh zero-file run
5. `L05` fresh one-file run
6. `L06` fresh three-file run
7. `L07` stale-chip clear/retry
8. `L08` send recovery after click/receipt-loss injection
9. `L09` poll running to terminal
10. `L10` show pinned stopped runtime
11. `L11` resume pinned stopped runtime
12. `L12` download optional zero
13. `L13` download required one visible control with a real Playwright event
14. `L14` download current-turn N controls
15. `L15` unknown session
16. `L16` root/mismatched conversation URL
17. `L17` cohort fairness with unavailable slot
18. `L18` provider-limit cooldown and rotation
19. `L19` restart/replay after terminal before output
20. `L20` release takeover/recovery
21. `L21` final status/cleanup

For focused investigation, run exactly one complete-matrix case:

```bash
bash scripts/qa-live-matrix-r13.sh \
  --case L03 \
  --source-fingerprint "$SOURCE_FINGERPRINT"
```

Single-case mode never substitutes for the three complete iterations.

## Targeted R01-R10 repetition

Only the following timing/race/restart/cooldown/fairness/unknown-session/
release-recovery cases have `repeat10=true`:

1. `R01` concurrent allocator no-overlap and exact cursor advance
2. `R02` readiness-failure/unavailable-slot fairness rotation
3. `R03` claim/lease/runtime renewal across simulated and real long operation
4. `R04` send click crash/read-only reconciliation
5. `R05` artifact consumed crash/no-reclick recovery
6. `R06` journal/HEAD/projection restart failpoints
7. `R07` provider-limit cooldown expiry/clear/reallocation
8. `R08` unknown-session concurrent resume/show/download
9. `R09` tokenless release takeover race
10. `R10` release cleanup interruption and exactly-once resource release

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

A failed or out-of-order repetition resets that case. Deterministic, static,
and schema cases receive no arbitrary ten-run requirement.

## Evidence required for every live case

Every case records command argv/environment names/cwd, exit code, exact
stdout/stderr hashes, source fingerprint, provider request and immutable receipt
IDs, journal event/projection manifests, and the post-case release result. Cases
that touch the browser also record privacy-safe screenshots and sanitized
DOM/CDP from the pinned slot before each wait or mutation.

Where applicable, evidence additionally binds request/run/session IDs, the real
non-root conversation URL, slot/cohort, user and assistant turn IDs, attachment
set digest, upload proof, terminal answer bytes and SHA-256, artifact claim and
control IDs, the listener-before-click timestamp, real Playwright download
event, host path, nonzero size, SHA-256, and MIME/type.

Provider-limit evidence comes only from a visible blocking modal, toast, alert,
or dialog. Matching words in prompt/answer/sidebar/history/attachment/composer
content are not provider state. Initial non-Pro composer state is picker-
correctable; it is not an allocation failure before bounded picker selection
and fresh verification.

## Mandatory release oracle

Every pass, failure, timeout, and interrupted-recovery branch performs
evidence-preserving release. Completion requires:

- request and session-operation claims released exactly once when acquired;
- slot lease and runtime ownership released exactly once when acquired;
- runtime stop authorized by the current owner or a committed dead-owner
  takeover proof; a live or unknown owner produces an explicit stop-skip;
- `holders=0`, `locks=0`, and no live lifecycle-owned runtime;
- the used slot is allocatable standby, exited, or explicitly cooldown-blocked;
  a stopped runtime is never reported as ready.

Do not use ad hoc Docker stops, Chrome kills, profile deletion, destructive Git
commands, or direct PR merge as cleanup. PR #72 remains open, draft, and
unmerged. One fresh independent implementation review over unchanged final
source/diff/evidence with exact `VERDICT: LGTM_NO_BLOCKING` is the review gate;
ten repeated reviews are forbidden.
