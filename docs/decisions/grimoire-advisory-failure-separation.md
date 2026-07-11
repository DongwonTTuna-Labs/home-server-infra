# ADR: Grimoire Advisory And Failure Separation

## Status

Accepted for the v1 advisory and failure separation behavior. This records completed cast, design, spec-gap, labels, action-output, scope-manifest, fixture, and doc behavior. It doesn't claim production rollout or live cross-repo execution evidence.

## Context

The first reusable Grimoire control plane treated missing OpenSpec evidence as a hard halt in the docs and early fixtures. The redesigned implementation now has three outcomes: `success`, `neutral`, and `failure`.

Specs are still expected. The change is that some missing-evidence cases are advisory and non-blocking for the current PR event. Real safety violations still fail red. Protected paths, unsafe targets, malformed severities, malformed verdicts, scope violations, missing credentials, boulder failures, push failures, and high-safety missing evidence remain fail-closed paths.

The compatible `spec-gap-halt` decision string stays in place so downstream readers don't break. Its cast conclusion is now `neutral`, with exit 0 and no push. The spec-gap status still carries compatibility fields, but marks the event advisory and includes `no_code_or_push_action=true`.

## Decision

Grimoire uses this outcome taxonomy:

| Conclusion | Meaning | Final status | Exit |
| --- | --- | --- | --- |
| `success` | Clear terminal no-op or one scoped push awaiting `pull_request.synchronize` | `terminal` or `awaiting-synchronize` | 0 |
| `neutral` | Spec-gap advisory or strict no-actionable-work | `advisory` | 0 |
| `failure` | Safety, contract, credential, scope, verdict, boulder, or push failure | `fizzled` | 1 |

`spec-gap-halt` keeps the compatible decision string and now carries `conclusion="neutral"`, `exit_code=0`, `status="ok"`, label intent `spec-needed`, `should_push=false`, and `terminal=false`.

`spec-gap` produces a five-section advisory comment and status with `advisory=true` and `no_code_or_push_action=true`. It doesn't authorize code changes, file writes, commits, or push. The guidance is satisfied by adding or updating truthful OpenSpec evidence and rerunning Grimoire through the normal pull request event path.

The labels action manages `📋 Spec Needed` as display-only advisory state. The `spec-needed` transition removes managed running, done, and fizzled labels, adds `📋 Spec Needed`, and preserves unrelated labels. Labels aren't workflow machine state.

The optional `.omo/grimoire/scope.yml` v1 manifest can narrow governed path evaluation with safe relative `governed_paths` and `advisory_only_paths` globs. A valid manifest gates governed paths only. It can't expand write authority, can't bypass protected-path or scope guards, and can't approve unsafe targets. Absent, malformed, or invalid manifests fall back to severity-threshold behavior.

The cast action exposes `conclusion` and `summary` outputs for optional display or presentation layers. There is no separate neutral check-run in this release.

## Alternatives

1. Add an opt-in trigger for advisory mode. Rejected because Grimoire has no manual dispatch or runtime control surface, and per-consumer triggers would create policy drift.
2. Disable Grimoire when spec-gap appears. Rejected because it would hide useful OpenSpec guidance, leave humans without a clear next step, and make label state harder to read.
3. Keep all spec-gap paths red. Rejected because info, low, medium, ungoverned, and advisory-only findings need guidance without blocking the PR event when no safe code action is allowed.

## Backward-Safety

Consumer workflow shape now includes the guarded `pull_request.unlabeled` path for `📋 Spec Needed` removals. Consumers still call `DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main`, keep `permissions: {}`, map named secrets explicitly, and use GitHub App installation-token auth through `GRIMOIRE_APP_PRIVATE_KEY` plus `grimoire_app_client_id`. `docs/decisions/grimoire-app-auth.md` records the current auth decision.

The compatible `spec-gap-halt` decision string stays available for readers that key on the old name. The new `conclusion` field is the authoritative outcome class for success, neutral, and failure.

Safety gates are not weakened. Protected paths, unsafe target paths, malformed inputs, scope violations, missing credentials, rejected F1-F4 verdicts, bad push counts, and failed pushes still produce `failure`, `fizzled`, and exit 1.

The scope manifest can't expand authority. It only narrows how design gates governed paths, and invalid manifests fall back to severity-threshold behavior.

## Consequences

PR owners get a green process result for advisory spec guidance, plus a clear `📋 Spec Needed` display label when OpenSpec evidence is needed.

Automation should read `conclusion`, `summary`, and status artifacts. It should not infer machine state from display labels.

Docs, ADRs, release notes, and doc-contract tests now describe the advisory/failure split so future changes keep spec guidance separate from red safety failures.
