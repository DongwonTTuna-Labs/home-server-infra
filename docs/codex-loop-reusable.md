# Codex Loop Reusable Workflow Contract

## Event Topology

This contract is for org-internal Codex loop automation in `DongwonTTuna-Labs` repositories. It defines one reusable core and one event adapter.

The reusable core is `codex-loop-reusable.yml` with `on: workflow_call`. It owns validation, trust checks, Codex relay setup, stage execution, and terminal decisions. Consumer repositories call it directly when they already have trusted PR context.

The event adapter is `codex-loop-dispatch.yml` with `on: repository_dispatch` and `types: [codex-loop]`. It runs on the default branch, validates `client_payload`, maps the payload into reusable workflow inputs, and calls the reusable core. It doesn't hold extra loop state.

The manual adapter is `codex-loop-manual.yml` with `on: workflow_dispatch`. It is a debug-only entry point, defaults `dry_run` to `true`, requires the same PR identity inputs including `head_sha`, and is not the normal loop progression mechanism.

The core and adapter run on the `Home Server Runners` org runner group with the `dongwontuna-labs-runner` label unless a consuming repo explicitly passes another approved org-internal runner target in a later change.

Labels and comments MUST NOT be used for internal loop state. For text checks, labels and comments MUST NOT be used for internal loop state. The loop state lives in typed workflow inputs, `repository_dispatch` payload, job outputs, summaries, and artifacts.

## Payload Schema

Use `event_type: codex-loop`. The current dispatch contract is payload schema v2. `client_payload` must contain these top-level keys:

```json
{
  "schema_version": 2,
  "pr_number": 123,
  "head_sha": "40-char-commit-sha",
  "base_ref": "main",
  "stage": "review",
  "iteration": 0,
  "correlation_id": "codex-loop-123-<headsha-prefix>",
  "requested_by": "github-login-or-app-slug",
  "state_run_id": 1234567890,
  "state_artifact_name": "codex-loop-state-codex-loop-123-<headsha-prefix>"
}
```

The JSON Schema lives at `schemas/codex-loop-dispatch-payload.v2.schema.json`. It sets `additionalProperties: false`, so callers must not add labels, comments, model output, secrets, or arbitrary free-form state to `client_payload`.

Field contract:

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `schema_version` | integer enum | yes | Must be `2`. This makes state-pointer payloads distinguishable from the original PR #18 payload shape. |
| `pr_number` | integer | yes | Pull request number in the repository that receives the event. |
| `head_sha` | string | yes | Expected PR head commit SHA. The core must compare this with the live PR head before any write, finalize, or continuation operation. |
| `base_ref` | string | yes | Base branch name used for trust and checkout decisions. |
| `stage` | string enum | yes | One of `review`, `design`, `fix`, or `issue`. |
| `iteration` | integer | yes | Zero-based loop counter for dispatches that entered the state machine. The default maximum is `5`. |
| `correlation_id` | string | yes | Stable id for this loop chain, used in concurrency groups, summaries, artifacts, and state artifact names. |
| `requested_by` | string | yes | GitHub login or app slug that requested this loop step. |
| `state_run_id` | integer | yes | GitHub Actions run id that produced the state artifact to load before executing this step. The first adapter-created dispatch points at the run that initialized the state bundle. |
| `state_artifact_name` | string | yes | Exact artifact name containing the state bundle for this loop step. It must be resolved by name, not by `gh run list --headSha` discovery. |
| `dry_run` | boolean | no | Defaults to `true` for the core and adapters. |
| `max_iterations` | integer | no | Defaults to `5`; see the cap semantics below. |

No free-form state is allowed in the payload. If the loop needs more state later, add a new versioned schema in a separate change.

## Inputs

The reusable core accepts the same contract as typed `workflow_call` inputs:

| Input | Type | Required | Notes |
| --- | --- | --- | --- |
| `pr_number` | number | yes | Source PR number. |
| `head_sha` | string | yes | Expected PR head SHA. |
| `base_ref` | string | yes | Expected base ref. |
| `stage` | string | yes | Must be `review`, `design`, `fix`, or `issue`. |
| `iteration` | number | yes | Must be within the accepted loop range. |
| `correlation_id` | string | yes | Used to tie runs, artifacts, and dispatches together. |
| `requested_by` | string | yes | Used for audit text and trust checks. |
| `dry_run` | boolean | no | Defaults to `true` for the core and adapters. A `true` value forces non-writing behavior even if live support exists. |
| `enable_live_autofix` | boolean | no | Defaults to `false`. Live model calls, relay-token minting, GitHub App-token minting, fix pushes, and continuation dispatch require `dry_run == false && enable_live_autofix == true`. |
| `max_iterations` | number | no | Defaults to `5`. |

## Org Consumer Adapters

Org-internal consumers should add a thin adapter in the consuming repository. The adapter owns the event trigger and passes trusted PR context into the reusable core. The reusable core stays in `home-server-infra`.

Production consumers must pin the reusable workflow by commit SHA or by an approved internal tag. Don't pin production consumers to a branch, because a moving ref can change the automation that runs against PR code without a consumer-side review.

A safe first `pull_request_target` adapter can start in dry-run mode and call only the `review` stage:

```yaml
name: Codex Loop Review Adapter

on:
  pull_request_target:
    types: [opened, synchronize, reopened, ready_for_review]

permissions: {}

jobs:
  codex-loop-review:
    if: ${{ github.event.pull_request.draft == false }}
    uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@<sha-or-tag>
    permissions:
      contents: write
      pull-requests: write
      id-token: write
    with:
      pr_number: ${{ github.event.pull_request.number }}
      head_sha: ${{ github.event.pull_request.head.sha }}
      base_ref: ${{ github.event.pull_request.base.ref }}
      stage: review
      iteration: 0
      correlation_id: codex-loop-${{ github.event.pull_request.number }}-${{ github.event.pull_request.head.sha }}-0
      requested_by: ${{ github.actor }}
      dry_run: true
    secrets: inherit
```

The `dry_run: true` default is the safe initial path for a new consumer. Switch it only after the consumer has reviewed trust checks, runner access, app secrets, and artifact output for that repository.

Live autofix is default-off in two independent ways: `dry_run` defaults to `true`, and `enable_live_autofix` defaults to `false`. A caller enables live behavior only by passing both `dry_run: false` and `enable_live_autofix: true`. Setting `dry_run: false` alone is still non-live/default-off; model stages use deterministic no-model artifacts, relay-token steps are skipped, GitHub App-token steps are skipped, push is skipped, and continuation dispatch is skipped.

Initial review bootstrap is allowed only for `stage=review`, `iteration=0`, and both `state_run_id` and `state_artifact_name` empty. That first-review path creates a run-scoped empty loop-state bundle. Every resume or non-initial stage still requires explicit state pointers.

The manual debug adapter is dry-run oriented. In its current shape it requires state pointers and does not expose `enable_live_autofix`, so it cannot be used as a no-state initial-review live-enable wrapper.

Trusted fix-push continuation is now implemented behind the live gates. If Codex returns `should_redispatch=true`, a push and redispatch may occur only after same-repository trust checks, non-fork/stale-head guards, semantic safety validation, `dry_run == false`, and `enable_live_autofix == true`. Without both live flags the run terminates as non-live/default-off with no push or dispatch.

The dispatch payload keys are `schema_version`, `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, `requested_by`, `state_run_id`, `state_artifact_name`, and optional `dry_run` or `max_iterations`. They are used by the default-branch adapter and by future trusted continuation support.

## Label Migration

The old label-driven chain maps to typed stages like this:

| Old label signal | New stage path |
| --- | --- |
| `리뷰중` | Dispatch or reusable input `stage: review`. |
| `리뷰완료` | Dispatch or reusable input `stage: design`. |
| `설계완료` | Dispatch or reusable input `stage: fix`. |
| `수정중` | Dispatch or reusable input `stage: fix`. |
| `codex:needs-issue` | Dispatch or reusable input `stage: issue`, or a terminal artifact path until issue creation is added in a separate change. |
| `codex:lgtm` | Optional future visible marker only. It is not internal loop state. |

Internal labels `리뷰중`, `리뷰완료`, `설계완료`, and `수정중` are removed as orchestration triggers. Don't add or remove those labels to drive the loop. The source of truth is the typed workflow input or `repository_dispatch` payload.

Terminal visible labels or comments can be considered later as visible-status work. They are optional, separate from this loop contract, and must not become internal state.

## Secrets

The reusable accepts these GitHub App secret names only. Do not put secret values in workflow YAML, docs, evidence, logs, PR bodies, or summaries.

| Secret | Required For | Notes |
| --- | --- | --- |
| `CODEX_GITHUB_APP_ID` | live fix push and live continuation dispatch | GitHub App id for the org-internal automation app. Required only when `dry_run == false && enable_live_autofix == true` reaches a write or dispatch path. |
| `CODEX_GITHUB_APP_PRIVATE_KEY` | live fix push and live continuation dispatch | PEM private key for the same app. Required only when `dry_run == false && enable_live_autofix == true` reaches a write or dispatch path. |

Consumer repositories must map their own secret names explicitly. The RS consumer will map `CODEX_APP_ID` to `CODEX_GITHUB_APP_ID` and `CODEX_APP_PRIVATE_KEY` to `CODEX_GITHUB_APP_PRIVATE_KEY` in a later adapter change; `secrets: inherit` does not rename them.

PAT fallback is forbidden. Workflows must not accept, document, or branch to a PAT for write, finalize, or dispatch operations.

Codex model access uses GitHub OIDC and the org native `codex-lb` relay path. Jobs that need the relay grant `id-token: write`, exchange the OIDC token for a short-lived relay key through `codex-review oidc relay-token`, mask the key, and pass the relay endpoint and key to Codex. No long-lived relay key is stored in GitHub secrets.

## Permissions

Use explicit least-privilege permissions. Never use `write-all`.

| Area | Required Permissions | Notes |
| --- | --- | --- |
| Dispatch adapter validation | `contents: read`, `pull-requests: read` | No `id-token` and no write permission. |
| Core validation and trust checks | `contents: read`, `pull-requests: read` | Runs before relay setup or writes. |
| Core relay and model run | `contents: read`, `pull-requests: read`, `id-token: write` | Needed for OIDC relay exchange. |
| Core write and continuation | `contents: write`, `pull-requests: write` | Used only after trust guards and live gates pass, with GitHub App installation tokens. |

No issue permission is included in this change. Terminal failures go to job summary and artifacts, not issue creation, PR comments, or labels.

## Trust Boundary

The trusted boundary is the `DongwonTTuna-Labs` organization plus the GitHub App and OIDC relay trust configured for this infrastructure. A job must validate PR identity, current head SHA, base ref, actor, repository owner, and fork status before relay setup, checkout of PR code, writes, or continuation dispatch.

Fork PRs are not write-capable in this contract. If a fork PR reaches the loop, it must terminate before relay setup or write operations with a clear terminal reason in the job summary and artifact.

Same-repo fix pushes are live-gated. The reusable workflow validates trust and stage output, requires `dry_run == false && enable_live_autofix == true`, pushes only trusted same-repo branches, emits an `updated_head_sha`, and dispatches that updated SHA with the GitHub App installation token. Dry-run/default-off paths do not push or redispatch.

### Checkout Topology

The reusable core separates trusted code from untrusted PR data using four named worktrees. Each worktree has a fixed repository/ref source, credential policy, trust level, and allowed-action contract. A job must never collapse these into a single checkout, because that would let untrusted PR-head content run with trusted credentials.

| Worktree | Repository/ref | Credential policy | Trust level | Allowed actions | Forbidden actions |
| --- | --- | --- | --- | --- | --- |
| `trusted-core/` | `DongwonTTuna-Labs/home-server-infra@<pinned SHA>`, the same SHA the consumer pinned in `uses:`, resolved from `github.workflow_ref` or an explicit `core_sha` assertion | Trusted org checkout; carries no PR-head write credentials | Trusted | Install and run the trusted `codex-review` CLI and trusted helper scripts | Checking out or executing PR-head code in this worktree |
| `target-base/` | Consumer repository base ref or base SHA (`base_ref`) | `persist-credentials: false` | Trusted comparison/reference context | Read-only base context for diff and reference against the PR head | Writing, pushing, or executing as a privileged step |
| `pr-head/` | Consumer PR head SHA (`head_sha`) | `persist-credentials: false` | Untrusted PR-head data | Read PR-head files as data for review and diff only | Executing PR-head scripts, actions, package managers, or helper installation under secrets |
| `pr-head-write/` | Consumer PR head SHA, fresh checkout taken only after same-repo, fork, and stale-head guards pass | Credentials limited to the GitHub App installation token | Write-capable only after trust guards pass (future fix-push work) | Apply a verified fix and push to a trusted same-repo branch | Fork PRs, stale heads, or executing arbitrary PR-head scripts under secrets |

`PR head is data only; never execute PR-head scripts, actions, package managers, or helper installation under secrets.` The `pr-head/` worktree exists so the trusted `codex-review` CLI in `trusted-core/` can read the proposed change as text. It is never a place to run the consumer's build, install, test, or lint commands while org secrets or the GitHub App token are in scope.

`pr-head-write/` is a separate, fresh checkout taken only after the same-repo, fork, stale-head, semantic-safety, and live-enable guards pass. It carries only the GitHub App installation token, and never reuses the `pr-head/` data worktree.

CLI SHA sourcing rule: the reusable core must source the `codex-review` CLI from the same pinned `home-server-infra` SHA as the workflow itself. The core resolves that SHA from `github.workflow_ref` (parsed) or asserts an explicit `core_sha` input in later implementation, then checks out `trusted-core/` at that exact SHA. A branch pin or any other moving ref is forbidden for production, because a moving core ref could change the trusted CLI that runs against PR data without a pinned, reviewable SHA.

## Loop State Machine

The loop has four non-terminal stages: `review`, `design`, `fix`, and `issue`.

Allowed non-terminal transitions:

| Current stage | Allowed next stage | Meaning |
| --- | --- | --- |
| `review` | `design` | Review found changes that need a design pass before modification. |
| `design` | `fix` | Design is complete and the loop may apply the scoped fix. |
| `fix` | `review` | A trusted fix commit was produced, the PR head moved, and the next iteration must review the updated head. |
| `review` | `issue` | Review found a condition that should become a separately tracked issue instead of continuing the PR loop. |
| `design` | `issue` | Design found a scope or requirement problem that should terminate into the issue path. |
| `fix` | `issue` | Fix execution found a non-recoverable implementation problem that should terminate into the issue path. |

Terminal states are not payload stages. They are workflow outputs, summaries, and artifacts only:

| Terminal state | Required output shape | Meaning |
| --- | --- | --- |
| LGTM | `lgtm=true`, `should_redispatch=false`, `terminal_reason=lgtm` or an equivalent success reason | The PR is ready and no further design, fix, issue, or dispatch step is required. |
| Issue terminal | `lgtm=false`, `should_redispatch=false`, `terminal_reason` prefixed with `issue-` or another explicit issue-stage reason | The loop stops in the issue path. This contract does not create issues, comments, or labels. |
| Failure terminal | `lgtm=false`, `should_redispatch=false`, non-empty `terminal_reason` | Validation, trust, stale-head, max-iteration, relay, ambiguous-output, fork, closed-PR, or write failure stopped the loop. |

A valid continuation dispatch must carry the next `stage`, the updated `iteration`, the expected `head_sha`, and the explicit state pointers `state_run_id` and `state_artifact_name`. The next run must load that artifact before stage execution. It must not infer prior state from labels, comments, issues, or `gh run list --headSha`.

### Dispatch And Iteration Caps

`iteration` is zero-based. The first review dispatch for a head starts at `0`. A continuation that moves from `fix` back to `review` after a trusted same-repo fix push increments `iteration` by `1` and must carry the updated PR `head_sha`.

`max_iterations` defaults to `5`. A run where `iteration >= max_iterations` must not dispatch another loop step. It must terminate with a max-iteration terminal reason before any write or redispatch attempt. Validation may accept the run for summary/artifact publication, but finalization must keep `should_redispatch=false`.

Each continuation attempt must obey both caps:

| Cap | Rule |
| --- | --- |
| Per-run dispatch cap | A single workflow run may emit at most one continuation dispatch. |
| Loop iteration cap | A loop chain may continue only while `iteration < max_iterations`. |

### Stale-Head Checkpoints

The stale-head guard runs at every trust boundary:

1. Dispatch validation records the payload `head_sha` and state pointers without resolving historical runs by head SHA.
2. Before relay setup, checkout, writes, state artifact loading, or stage execution, the core compares the live PR head with payload `head_sha`.
3. Before any fix push or continuation dispatch, the workflow checks the PR head again. If a fix push created a new commit, the continuation payload must use that new SHA; if a user or another workflow moved the head unexpectedly, the run terminates with `stale-head-sha`.
4. The next run repeats the same guard before loading `state_artifact_name` from `state_run_id`.

A stale-head terminal is final for that run. It must be visible in summaries and artifacts, not labels or comments.

## Loop Termination

The loop has four stages: `review`, `design`, `fix`, and `issue`.

A successful terminal run happens when Codex returns a machine-readable result that the PR is ready and no further design, fix, or issue stage is required.

A continuing run may happen only after the `fix` stage pushes a trusted same-repo commit, emits `updated_head_sha`, and dispatches a new `stage=review` event with `iteration` increased by `1` and the updated `head_sha`. This path is live-gated and remains inert unless `dry_run == false && enable_live_autofix == true`.

A failure terminal run happens when validation fails, max iteration is exceeded, SHA is stale, the actor isn't trusted, required GitHub App secrets are missing, the payload is malformed, Codex output is ambiguous, the PR is closed or missing, the PR comes from a fork, or a write fails.

The canonical machine-readable enum is `schemas/terminal-reason.v1.json`. Workflow outputs, summaries, and artifacts should use only these underscore names after emitter-alignment work is complete.

| reason | meaning | originating stage/guard |
| --- | --- | --- |
| `lgtm` | The PR is accepted with no further loop action. | `review` stage result |
| `dry_run` | The run intentionally stopped before live writes or redispatch. | finalize dry-run guard |
| `no_fix_needed` | The requested fix stage found no implementation change was necessary. | `fix` stage result |
| `no_fix_changes` | The fix stage ran but produced no file diff to apply. | `fix` stage result |
| `empty_patch` | A generated patch was empty or could not alter the worktree. | patch application guard |
| `validation_failed` | Workflow inputs or dispatch payload failed validation. | dispatch/core validation guard |
| `tests_failed` | Required validation commands or tests failed. | post-fix verification |
| `semantic_safety_missing` | Required semantic-safety evidence was not present. | semantic safety guard |
| `semantic_safety_rejected` | Semantic-safety validation rejected the proposed change. | semantic safety guard |
| `semantic_safety_hash_mismatch` | Safety evidence did not match the expected artifact hash. | semantic safety guard |
| `policy_rejected` | Repository or org policy blocked continuation. | policy guard |
| `stale_head` | The live PR head no longer matches the expected `head_sha`. | stale-head guard |
| `base_ref_mismatch` | The live PR base ref no longer matches the expected `base_ref`. | trust and stale guard |
| `pr_closed` | The PR is closed or unavailable for loop processing. | trust and stale guard |
| `fork_pr` | The PR head is from a fork and is not write-capable. | trust and stale guard |
| `untrusted_repository_owner` | The repository owner is outside the trusted org boundary. | trust guard |
| `untrusted_requester` | The requester is not an accepted actor for the PR. | trust guard |
| `missing_app_credentials` | Required GitHub App credentials are absent for a live write path. | app token setup guard |
| `app_token_scope_invalid` | The GitHub App token lacks the required repository permissions. | app token setup guard |
| `push_failed` | A trusted fix push or push-capable continuation failed. | fix push guard |
| `pushed_unverified` | A push completed but the updated head could not be verified. | post-push stale-head guard |
| `dispatch_failed` | A continuation `repository_dispatch` request failed. | dispatch guard |
| `dispatch_duplicate` | A duplicate continuation dispatch was detected or suppressed. | dispatch guard |
| `max_iterations` | The loop reached `iteration >= max_iterations`. | iteration cap guard |
| `oscillation_detected` | The loop detected repeated non-progressing states. | loop progress guard |
| `artifact_missing` | A required state or stage artifact was missing. | artifact load guard |
| `artifact_schema_invalid` | A required artifact existed but failed schema validation. | artifact validation guard |
| `model_output_invalid` | Codex output was missing, malformed, or internally inconsistent. | stage result parser |
| `stage_failed` | A stage failed without a more specific terminal reason. | stage execution |
| `issue_created` | Future optional issue path completed by creating an issue. | `issue` stage result, future optional |

Current emitter alignment note: existing workflows still emit some hyphenated or placeholder strings. Do not add new workflow strings; align them in later emitter tasks as follows.

| current workflow string | canonical reason |
| --- | --- |
| `dry-run` | `dry_run` |
| `dry-run-placeholder` | `dry_run` |
| `max-iterations-exceeded` | `max_iterations` |
| `stale-head-sha` | `stale_head` |
| `base-ref-mismatch` | `base_ref_mismatch` |
| `pr-closed` | `pr_closed` |
| `fork-pr` | `fork_pr` |
| `untrusted-repository-owner` | `untrusted_repository_owner` |
| `untrusted-requester` | `untrusted_requester` |
| `trusted-fix-push-not-implemented` | legacy placeholder; do not emit for live-capable fix-push paths |
| `missing-*`, `invalid-*`, `validation-failed` | `validation_failed` |
| `invalid-pr_number`, `invalid-iteration`, `invalid-max_iterations`, `invalid-stage`, `invalid-dry_run` | `validation_failed` |
| `relay-token-empty`, `relay-setup-failed` | `missing_app_credentials` |
| `missing-stage-result-json` | `artifact_missing` |
| `terminal-reason-required`, `invalid-next-stage`, `redispatch-missing-next-stage` | `model_output_invalid` |
| `stage-failed`, `issue-stage-placeholder` | `stage_failed` |
| `trust-or-stale-guard-failed` | `policy_rejected` |

Terminal failures must be visible in job summary and artifacts. Labels and comments MUST NOT be used for internal loop state, including terminal failure state.

### Terminal Visibility Artifact And Conclusion

`finalize-stage` always publishes a `codex-loop-terminal-<correlation_id>-<iteration>` artifact containing a machine-readable `codex-loop-state.json` and a human-readable `terminal-summary.md`, and mirrors the human summary into `$GITHUB_STEP_SUMMARY`. The decision is sourced from the canonical `effective_outputs` step (the post-guard source of truth), not the pre-normalization `finalize` step.

`codex-loop-state.json` carries at least `schema_version`, `stage`, `pr_number`, `head_sha`, `base_ref`, `correlation_id`, `iteration`, `terminal_reason`, `lgtm`, `should_redispatch`, `next_stage`, `state_run_id`, `state_artifact_name`, `updated_head_sha`, `dry_run`, and `selected_result`. `terminal-summary.md` states the terminal reason, the next manual action, the selected state artifact plus a `gh run download` lookup hint, and an explicit note that no labels, comments, or issues hold loop state. Neither file contains secrets, tokens, relay material, or raw model output.

The workflow conclusion reflects the terminal outcome. These reasons keep the run green: an empty reason (LGTM terminal or in-progress continuation), `lgtm`, `dry_run`, `no_fix_needed`, `no_fix_changes`, `empty_patch`, and `issue_created`. Every other canonical `terminal_reason` (for example `validation_failed`, `stale_head`, `base_ref_mismatch`, `pr_closed`, `fork_pr`, `untrusted_repository_owner`, `untrusted_requester`, `missing_app_credentials`, `app_token_scope_invalid`, `tests_failed`, `semantic_safety_*`, `policy_rejected`, `push_failed`, `pushed_unverified`, `dispatch_failed`, `dispatch_duplicate`, `max_iterations`, `oscillation_detected`, `artifact_missing`, `artifact_schema_invalid`, `model_output_invalid`, `stage_failed`) fails the run via a non-zero exit in `finalize-stage`. Failure terminals are never masked as a successful continuation, and no broad `continue-on-error` covers them.

## Rollout

Start with a SHA-pinned org-internal consumer on one trusted repository. Use `stage=review`, `iteration=0`, a unique `correlation_id`, no state pointers, `dry_run: true`, and the default `enable_live_autofix: false` for the first run. Keep the first consumer in dry-run/default-off mode until workflow summaries and artifacts prove the loop can read PR context, validate trust, bootstrap the initial review state, and terminate without writes.

`repository_dispatch` only starts workflows that already exist on the repository default branch. Merge `.github/workflows/codex-loop-dispatch.yml` to the default branch before running the dispatch smoke test. A branch-only adapter won't receive `repos/:owner/:repo/dispatches` events.

Keep the existing visible label/comment workflows disabled or separate while the new loop is tested. Don't run both orchestration systems on the same PR.

### Default-Branch Dispatch Smoke Test

Do not run this by default. Run it only when there is an explicit dry-run test PR and approval to send a live `repository_dispatch` event.

Smoke test prerequisites:

| Placeholder | Required value |
| --- | --- |
| `<owner>` | Repository owner, normally `DongwonTTuna-Labs`. |
| `<repo>` | Repository that already has `.github/workflows/codex-loop-dispatch.yml` on its default branch. |
| `<pr-number>` | Open same-repository dry-run test PR number. |
| `<head-sha>` | Current head SHA for that PR. |
| `<base-ref>` | Base branch for that PR, for example `main`. |
| `<iteration>` | Usually `0` for the first smoke run. |
| `<requested-by>` | GitHub login or GitHub App slug requesting the smoke run. |
| `<state-run-id>` | GitHub Actions run id that produced the state artifact for this dispatch. |
| `<state-artifact-name>` | Exact state artifact name to load for this dispatch. |
| `<max-iterations>` | Optional limit, usually `1` for a smoke run. |

Confirm the PR identity before dispatching:

```bash
gh pr view <pr-number> \
  --repo <owner>/<repo> \
  --json number,state,headRefOid,baseRefName,headRepositoryOwner,headRepository
```

The PR should be open, same-repository, and dry-run only. The `headRefOid` must equal `<head-sha>`, and `baseRefName` must equal `<base-ref>`.

Dispatch payload schema v2 smoke command:

```bash
gh api repos/:owner/:repo/dispatches \
  --method POST \
  --field event_type=codex-loop \
  --field client_payload[schema_version]=2 \
  --field client_payload[pr_number]=<pr-number> \
  --field client_payload[head_sha]=<head-sha> \
  --field client_payload[base_ref]=<base-ref> \
  --field client_payload[stage]=review \
  --field client_payload[iteration]=<iteration> \
  --field client_payload[correlation_id]=codex-loop-<pr-number>-<head-sha> \
  --field client_payload[requested_by]=<requested-by> \
  --field client_payload[state_run_id]=<state-run-id> \
  --field client_payload[state_artifact_name]=<state-artifact-name> \
  --field client_payload[dry_run]=true \
  --field client_payload[max_iterations]=<max-iterations>
```

The required v2 `client_payload` keys are `schema_version`, `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, `requested_by`, `state_run_id`, and `state_artifact_name`. Optional keys are `dry_run` and `max_iterations`. Keep `dry_run=true` for smoke tests.

Collect evidence after dispatch:

```bash
gh run list \
  --repo <owner>/<repo> \
  --workflow codex-loop-dispatch.yml \
  --json databaseId,displayTitle,event,headBranch,headSha,status,conclusion,createdAt \
  --limit 10
```

```bash
gh run view <run-id> \
  --repo <owner>/<repo> \
  --json conclusion,jobs
```

If labels or comments could be affected by another workflow in the same repository, compare PR-visible state before and after the run without treating labels or comments as loop state:

```bash
gh pr view <pr-number> \
  --repo <owner>/<repo> \
  --json labels,comments,reviews,latestReviews
```

Expected smoke result:

1. The run event is `repository_dispatch` and the workflow is `Codex Loop Repository Dispatch`.
2. The dispatch validation summary records `event_type=codex-loop` and `github.event.client_payload`.
3. The reusable core receives the same `schema_version`, `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, `requested_by`, `state_run_id`, `state_artifact_name`, `dry_run`, and `max_iterations` values.
4. The run remains dry-run/default-off and doesn't mint a relay token, call `openai/codex-action`, mint a GitHub App installation token, push, dispatch continuation, or create dirty label/comment state.
5. Any failure is terminal in the job summary or artifacts, not represented by labels or comments.

Verify the first rollout by checking the reusable workflow inputs, the dispatch adapter payload validation, the OIDC relay exchange path, the GitHub App token path for write-capable jobs, and the absence of label/comment state changes.

Expand to more repositories only after the first consumer records clean summaries, artifacts, and loop termination behavior.

## Rollback

Start with the least destructive stop: set `enable_live_autofix: false`, omit `enable_live_autofix`, or set `dry_run: true` in the consumer workflow call that invokes `DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@<sha-or-tag>`. Any of those changes returns the loop to non-live/default-off behavior without deleting shared workflow source.

If the shared workflow change itself is suspected, revert the HSI PR that introduced the live-capable reusable. If a consumer has already re-pinned to that HSI commit, re-pin the consumer back to the previous reviewed HSI SHA until the issue is understood.

If new entries must stop entirely, disable or remove the consumer workflow call. This stops new consumer-triggered runs without deleting shared workflow source.

If dispatch continuation is the failing path, disable the event adapter next. Either disable the `Codex Loop Repository Dispatch` workflow in repository settings or remove the `.github/workflows/codex-loop-dispatch.yml` trigger or adapter from the affected repository. Leave `codex-loop-reusable.yml` in place; without callers or dispatch adapters it is inert.

If a bad loop is already running, find active runs by `correlation_id` in run summaries, logs, or artifacts, then cancel matching active runs:

```bash
gh run list \
  --repo <owner>/<repo> \
  --workflow codex-loop-dispatch.yml \
  --json databaseId,displayTitle,status,conclusion,createdAt \
  --limit 50
```

```bash
gh run cancel <run-id> --repo <owner>/<repo>
```

Do not clean up by adding labels or comments as internal state markers.

If write behavior is suspected, rotate or suspend the GitHub App credentials before re-enabling consumers. Preserve run summaries, logs, and artifacts for diagnosis.

Known residual risk: the GitHub App installation and repository access are assumed installed until a live smoke test proves token minting and repository scope in the target repository.

Rollback order:

1. Set `enable_live_autofix: false`, omit it, or set `dry_run: true` in the consumer adapter.
2. Revert the HSI live-capable PR if the shared reusable is suspect.
3. Re-pin consumers to the previous reviewed HSI SHA if they had already adopted the live-capable SHA.
4. Disable or remove the consumer workflow call if new entries must stop entirely.
5. Disable or remove the `codex-loop-dispatch.yml` trigger or adapter if dispatch can still enter the loop.
6. Leave the reusable workflow source inert when there are no callers.
7. Cancel active runs tied to the affected `correlation_id`.
8. Rotate or suspend GitHub App credentials if unexpected write behavior is suspected.

## Non-Goals

This change doesn't package or describe external distribution.

This change doesn't allow PR labels or issue/PR comments as orchestration state. It MUST NOT use PR labels for internal loop state and MUST NOT use issue/PR comments for internal loop state.

This change doesn't add issue creation or issue permissions. The `issue` stage is a loop stage name only for now, and terminal failures stay in job summaries and artifacts.

This change doesn't add PAT fallback language or support.

This change doesn't make fork PRs write-capable.
