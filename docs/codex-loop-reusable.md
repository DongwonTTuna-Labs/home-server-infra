# Codex Loop Reusable Workflow Contract

This document describes the current Codex Loop implementation in `home-server-infra` after the Phase 1 memory and workflow modularization work. The workflow and tests are the source of truth:

1. `.github/workflows/codex-loop-reusable.yml`
2. `setup/codex-review/tests/workflow/test_simplified_core.py`
3. `.omo/evidence/gha-precedent-dossier.md`

The loop is private org infrastructure. It runs on the `Home Server Runners` runner group with the `dongwontuna-labs-runner` label.

## Current Topology

The reusable core is `.github/workflows/codex-loop-reusable.yml` with `on: workflow_call`. It currently has six jobs in this order:

| Job | Role | Current behavior |
| --- | --- | --- |
| `validate` | Core | Validates typed workflow inputs. |
| `setup-state` | Core | Checks out base and PR-head data, installs the trusted `codex-review` helper, and uploads the fresh loop-state artifact for this run. |
| `classify` | Control | Classifies whether the new commit is a trusted Codex memory-only commit. |
| `run-stage` | Core | Runs the live sequential model chain: review, design, fix, push. Later stages run only when earlier stage outputs request them. |
| `finalize` | Core | Resolves the final outcome, emits reusable workflow outputs, and dispatches the next iteration only after a remote-verified push. |
| `required-checks` | Control | Aggregates prior job results and fails only when a needed job failed or was cancelled. Skipped model work is allowed. |

The core jobs are `validate`, `setup-state`, `run-stage`, and `finalize`. The post-change control jobs are `classify` and `required-checks`.

## Final Module Map

The reusable workflow is now a thin orchestrator for the job DAG, job-level gates, runner permissions, credential capture, artifact transport, and trusted-source checkouts. Per-phase step logic lives in trusted local composite actions checked out from `${{ job.workflow_repository }}@${{ job.workflow_sha }}` under `trusted-core/` and invoked with static `./trusted-core/.github/actions/...` paths.

| Phase | Owning composite | Called from | Owns |
| --- | --- | --- | --- |
| Memory classify | `.github/actions/codex-memory-classify` | `classify` job | `codex-review loop memory-only-change`, classifier outputs, and classifier summary. |
| Common context | `.github/actions/codex-context` | `run-stage` job | PR context, changed lines, docs context, OpenSpec context, and memory context artifacts. |
| Review/techlead | `.github/actions/codex-review-phase` | `run-stage` job | Review prompt/model/validation, combined findings, techlead prompt/model/decision, review publication, and route-after-techlead outputs. |
| Design/design-chief | `.github/actions/codex-design-phase` | `run-stage` job | Design context, inventory, clusters, analysis collection, plan, design-chief prompt/model/decision, publication, and route outputs. |
| Fix/push | `.github/actions/codex-fix-phase` | `run-stage` job | Fix dispatch, fix agents, merge, semantic safety, push validation, trusted memory sidecar write, commit/push, and push completion outputs. |

The job-level workflow keeps the six jobs in order: `validate`, `setup-state`, `classify`, `run-stage`, `finalize`, and `required-checks`. The composites contain steps only; jobs, `needs`, runner labels, permissions, `if` gates, concurrency, and `workflow_call` inputs/outputs stay in the reusable workflow.

Trust sourcing is part of the contract: `pr-head` is data/working tree only. The reusable workflow must not call `pr-head/.github/actions/...` and must not use bare `./.github/actions/codex-*` paths.

The previous split-stage model is gone. There is no `stage` workflow input, no `state_run_id`, no `state_artifact_name`, no per-stage workflow fan-out, no matrix, and no cross-run state pointer contract. A single `run-stage` job executes `review -> design -> fix -> push` sequentially inside one workflow run. `repository_dispatch` is used only to start the next full iteration after a successful, remote-verified push.

## Inputs

The reusable core currently accepts these `workflow_call` inputs:

| Input | Type | Required | Meaning |
| --- | --- | --- | --- |
| `pr_number` | number | yes | Pull request number. |
| `head_sha` | string | yes | Expected PR head SHA for this iteration. |
| `base_ref` | string | yes | Expected base branch. |
| `iteration` | number | yes | Zero-based loop iteration. |
| `correlation_id` | string | yes | Stable loop id, used in concurrency and artifacts. |
| `requested_by` | string | yes | Actor or app slug that requested this iteration. |

Current continuation payloads carry only the next iteration entry inputs: `pr_number`, `head_sha`, `base_ref`, `iteration`, `correlation_id`, and `requested_by`. They do not carry stage names or state pointers.

## Runner Environment Credentials

The reusable workflow declares no `workflow_call` secrets and must not read `secrets.*` inputs. Credentials are runner environment variables on the self-hosted org runners:

| Runner env var | Used for |
| --- | --- |
| `CODEX_RELAY_API_KEY` | Static codex-lb key for `openai/codex-action@v1` via `https://relay-ai.dongwontuna.net/v1/responses`. The workflow masks it before passing it to the model action. |
| `CODEX_LOOP_PAT` | Push and continuation `repository_dispatch`. A PAT is the current implementation because memory commits and fix commits need follow-on workflows to run. |

This is not the old GitHub App or OIDC relay-token model. It is also not a PAT-forbidden model. `GITHUB_TOKEN` may still be used for read-only GitHub API context where the workflow already does that, but push and dispatch continuation use `CODEX_LOOP_PAT` from the runner environment.

## Single-Run Stage Flow

`run-stage` builds common context once under `codex-review-artifacts/common/`, including:

1. `pr-context.json`
2. `changed-lines.json`
3. `docs-context.md`
4. `openspec-context.json`
5. `memory-context.md`

The trusted helper builds `memory-context.md` with:

```bash
codex-review context memory \
  --pr-context codex-review-artifacts/common/pr-context.json \
  --repo-path pr-head \
  --out codex-review-artifacts/common/memory-context.md
```

That same memory context is passed to the nine current prompt-builder paths: review, techlead, design inventory, design clusters, design plan, design chief, fix dispatch agents, fix merge, and semantic safety.

The PR head checkout is data for trusted helper inspection and model working-directory use. Consumer workflows with privileged write-back must not run untrusted PR-head build scripts, package managers, actions, or helper installation while base-repo secrets or write tokens are in scope.

## Review Memory Contract

Review memory lives in PR-scoped files under:

```text
.omo/review-memory/pr-<n>/
```

Canonical generated files are:

| File | Meaning |
| --- | --- |
| `ledger.json` | Schema-valid `review-memory.v1` ledger. |
| `learnings.md` | Generated projection from ledger entries. |
| `decisions.md` | Generated projection from ledger entries. |
| `issues.md` | Generated projection from ledger entries. |
| `problems.md` | Generated projection from ledger entries. |

Memory is advisory PR-scoped context. Treat PR-branch memory as untrusted data unless a specific entry is trusted by provenance. It may inform prompts as background, but it must not route stages, force LGTM, suppress current findings by prompt text, decide stale-head safety, authorize writes, trigger redispatch, or replace current code, current OpenSpec, schemas, security rules, or system instructions.

Trusted resolved-finding memory can affect suppression only through the trusted exact-fingerprint path implemented in the helper, not through free-form prompt prose. Missing, corrupt, oversized, or untrusted memory must fail safe to advisory or empty context.

## Memory-Only No-Op Gate

The implemented no-op gate is Model A plus Option A:

1. No workflow-level `paths-ignore` is used for the reusable core.
2. `classify` runs after `setup-state` and before model work.
3. `classify` calls `codex-review loop memory-only-change` against the PR-head checkout.
4. A no-op requires all of these signals: changed paths are only review-memory paths, the head commit has the `codex-memory: true` marker, and the actor/requester guard matches configured own actors.
5. `run-stage` runs only when `needs.classify.outputs.should_run_model == 'true'`.
6. `finalize` emits `terminal_reason=memory_only_noop`, `should_redispatch=false`, and an empty `updated_head_sha` when the guarded no-op is accepted.
7. `required-checks` still succeeds when downstream model work is skipped.

The classifier is fail-open for model work. Missing marker, actor mismatch, invalid refs, mixed code and memory changes, and empty diffs keep `should_run_model=true`.

## Required Checks And Branch Protection

Consumer branch protection should require only the final aggregator job:

```text
required-checks
```

Do not require individual model or dispatch jobs. `run-stage` may legitimately skip for memory-only no-op commits, and skipped jobs report success under GitHub Actions semantics. The aggregator inspects `validate`, `setup-state`, `classify`, `run-stage`, and `finalize`; it fails only failed or cancelled dependencies.

## Consumer Trigger Guidance

Do not try to implement memory-only suppression with workflow-level `paths-ignore` in a consumer adapter.

`paths` and `paths-ignore` are documented for `push`, `pull_request`, and `pull_request_target` path filters. They do not provide a general solution for `repository_dispatch`, `workflow_call`, or `workflow_dispatch`, and skipped required workflows can leave required checks pending. For Codex Loop memory commits, keep the workflow running and use the in-workflow `classify` plus `required-checks` pattern.

`[skip ci]` and `skip-checks` apply to `push` and `pull_request` workflows. They do not stop `pull_request_target` or `repository_dispatch`. Do not rely on them to stop Codex Loop re-entry.

`GITHUB_TOKEN` has special non-recursion behavior for many events created by workflow actions. When a workflow needs a write to trigger follow-on workflows, use a GitHub App token or PAT. The current Codex Loop uses `CODEX_LOOP_PAT` for push and `repository_dispatch`, so assume memory commits can re-trigger consumers and keep the classifier guard enabled.

## Loop Guards

Current loop-guard layers are:

| Guard | Current contract |
| --- | --- |
| Actor guard | The classifier compares `github.actor`, `requested_by`, and `CODEX_LOOP_OWN_ACTORS`, defaulting to `github-actions[bot]`. |
| Commit marker | Memory-only no-op requires the `codex-memory: true` marker on the head commit. |
| PR-keyed concurrency | The reusable core uses `concurrency.group: codex-loop-${{ inputs.correlation_id }}`. Consumers should pass a PR-stable correlation id for the loop chain. |
| Required-check aggregator | Branch protection should require `required-checks`, not individual jobs that can skip. |
| Remote-verified push | Continuation dispatch occurs only after a push reports a verified updated head SHA. |

## PR-Scoped Non-Merge Contract

`.omo/` stays ignored on base branches. Review memory is not normal source code and should not land on `main` as part of merging a PR.

Trusted memory writers may force-add only canonical files under `.omo/review-memory/pr-<n>/` on the PR branch. They must not broaden autofix allowlists to let model patches edit memory directly.

Before merge, or in a guard job for protected branches, check the base branch with:

```bash
codex-review memory assert-not-on-base --base-ref main --repo-path <repo-path>
```

This command flags review-memory files present on base. A clean base exits successfully; a base branch containing `.omo/review-memory/pr-<n>/...` exits non-zero.

## Same-Repo And Fork Safety

Privileged write-back is for trusted same-repository contexts. Fork PRs and untrusted PR-head code require stricter handling:

1. Treat PR-head files as data under privileged workflows.
2. Do not execute PR-head scripts, package-manager lifecycle hooks, composite actions, or helper installation with base secrets or write tokens.
3. Use explicit PR-head checkout or push targets when writing to a PR branch.
4. Use `git add -f` only for canonical generated memory files that trusted code produced.
5. Keep `.omo/review-memory/**` out of model patch allowlists.

## Rollback And Disablement

To stop new Codex Loop entries, disable the consumer adapter or stop calling the reusable workflow. To stop continuation, remove access to `CODEX_LOOP_PAT` on the runner or disable dispatch-capable consumers. Preserve run summaries, artifacts, and memory files for diagnosis.

Do not use labels or comments as internal loop state during rollback. Loop state and outcomes live in workflow inputs, job outputs, summaries, artifacts, dispatch payloads, and PR-scoped advisory memory.

## Non-Goals

This document describes the implemented Task 24 through 28 modularization and should be updated if composite ownership changes.

This document does not reintroduce split-stage jobs, GitHub App secrets, OIDC relay-token exchange, or PAT-forbidden claims.

This document does not make memory trusted orchestration state.
