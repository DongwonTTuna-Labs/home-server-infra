# Codex Loop Serial Redesign Evidence And Rollback

This note records the Task 2-6 serial redesign contract for the HSI reusable workflow. It is documentation-only evidence for the PR from `feat/codex-loop-serial`; it does not enable live execution by itself.

## Simple Topology

The reusable workflow `.github/workflows/codex-loop-reusable.yml` intentionally has exactly five jobs:

1. `validate`
2. `trust-and-stale-guard`
3. `setup-relay`
4. `run-stage`
5. `finalize`

All stage behavior stays inside `run-stage`. The workflow must not add per-stage jobs, `strategy.matrix`, or `fromJson` fan-out.

## Stage Sequence

The callable stage sequence is `review -> design -> fix -> push -> loop`. The loop step is a bounded continuation back to `review` for the next iteration after `push` verifies a trusted same-repository update and emits an updated head SHA.

Dry-run paths materialize deterministic helper artifacts and summaries only. They must not mint relay tokens, call live model actions, mint GitHub App installation tokens, push commits, or send continuation dispatches.

## Live Enablement

Live behavior is enabled only when both inputs are set:

- `dry_run: false`
- `enable_live_autofix: true`

The live gates also require trusted same-repository PR state, non-fork status, non-stale head SHA, validated stage artifacts, semantic-safety evidence where applicable, and bounded finalize redispatch checks. `dry_run: false` by itself is insufficient, and `enable_live_autofix: true` by itself is insufficient.

## Required Secret Names

The reusable workflow accepts these secret names only for live write or continuation paths:

- `CODEX_GITHUB_APP_ID`
- `CODEX_GITHUB_APP_PRIVATE_KEY`

These are names only. Do not put secret values in workflow YAML, docs, evidence, logs, PR bodies, or summaries. Consumer repositories must map their own secret names explicitly when they adopt a pinned HSI SHA.

## Rollback

Use the least destructive rollback that stops the affected path:

1. Flag off live behavior by setting `enable_live_autofix: false`, omitting `enable_live_autofix`, or setting `dry_run: true` in consumer adapters.
2. Revert the HSI PR that introduced this serial redesign if the shared reusable is suspected.
3. Re-pin RS or another consumer repository to the prior reviewed HSI SHA until the issue is understood.
4. Disable the consumer adapter or default-branch dispatch adapter if new entries must stop entirely.
5. Cancel active runs for the affected `correlation_id` and preserve summaries/artifacts for diagnosis.

Do not use labels or comments as rollback state. Do not merge this PR without USER merge GATE B approval.
