# Codex Loop Reusable Workflow Contract

## Event Topology

This contract is for org-internal Codex loop automation in `DongwonTTuna-Labs` repositories. It defines one reusable core and one event adapter.

The reusable core is `codex-loop-reusable.yml` with `on: workflow_call`. It owns validation, trust checks, Codex relay setup, stage execution, and terminal decisions. Consumer repositories call it directly when they already have trusted PR context.

The event adapter is `codex-loop-dispatch.yml` with `on: repository_dispatch` and `types: [codex-loop]`. It runs on the default branch, validates `client_payload`, maps the payload into reusable workflow inputs, and calls the reusable core. It doesn't hold extra loop state.

The manual adapter is `codex-loop-manual.yml` with `on: workflow_dispatch`. It is a debug-only entry point, defaults `dry_run` to `true`, requires the same PR identity inputs including `head_sha`, and is not the normal loop progression mechanism.

The core and adapter run on the `Home Server Runners` org runner group with the `dongwontuna-labs-runner` label unless a consuming repo explicitly passes another approved org-internal runner target in a later change.

Labels and comments MUST NOT be used for internal loop state. For text checks, labels and comments MUST NOT be used for internal loop state. The loop state lives in typed workflow inputs, `repository_dispatch` payload, job outputs, summaries, and artifacts.

## Payload Schema

Use `event_type: codex-loop`. `client_payload` must contain these top-level keys:

```json
{
  "pr_number": 123,
  "head_sha": "40-char-commit-sha",
  "base_ref": "main",
  "stage": "review",
  "iteration": 0,
  "correlation_id": "codex-loop-123-<headsha-prefix>-0",
  "requested_by": "github-login-or-app-slug"
}
```

Field contract:

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `pr_number` | integer | yes | Pull request number in the repository that receives the event. |
| `head_sha` | string | yes | Expected PR head commit SHA. The core must compare this with the live PR head before any write or finalize operation. |
| `base_ref` | string | yes | Base branch name used for trust and checkout decisions. |
| `stage` | string enum | yes | One of `review`, `design`, `fix`, or `issue`. |
| `iteration` | integer | yes | Zero-based loop counter. The default maximum is `5`. |
| `correlation_id` | string | yes | Stable id for this loop chain, used in concurrency groups, summaries, and artifacts. |
| `requested_by` | string | yes | GitHub login or app slug that requested this loop step. |

No free-form state is allowed in the payload. If the loop needs more state later, add a versioned schema in a separate change.

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
| `dry_run` | boolean | no | Defaults to `true` for the core and adapters. |
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

Trusted fix-push continuation is intentionally disabled in this implementation. If Codex returns `should_redispatch=true`, the reusable core terminates with `terminal_reason=trusted-fix-push-not-implemented` instead of sending another event. A future change may enable continuation only after it adds a concrete same-repo checkout, commit, push, and `updated_head_sha` output, then dispatches that updated SHA with the GitHub App installation token.

The dispatch payload keys remain `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, `requested_by`, and optional `dry_run` or `max_iterations`. They are used by the default-branch adapter and by future trusted continuation support.

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

Future write and continuation jobs require a GitHub App installation token minted from these mandatory secrets:

| Secret | Required For | Notes |
| --- | --- | --- |
| `CODEX_GITHUB_APP_ID` | future fix push and continuation dispatch | GitHub App id for the org-internal automation app. |
| `CODEX_GITHUB_APP_PRIVATE_KEY` | future fix push and continuation dispatch | PEM private key for the same app. |

PAT fallback is forbidden. Workflows must not accept, document, or branch to a PAT for write, finalize, or dispatch operations.

Codex model access uses GitHub OIDC and the org native `codex-lb` relay path. Jobs that need the relay grant `id-token: write`, exchange the OIDC token for a short-lived relay key through `codex-review oidc relay-token`, mask the key, and pass the relay endpoint and key to Codex. No long-lived relay key is stored in GitHub secrets.

## Permissions

Use explicit least-privilege permissions. Never use `write-all`.

| Area | Required Permissions | Notes |
| --- | --- | --- |
| Dispatch adapter validation | `contents: read`, `pull-requests: read` | No `id-token` and no write permission. |
| Core validation and trust checks | `contents: read`, `pull-requests: read` | Runs before relay setup or writes. |
| Core relay and model run | `contents: read`, `pull-requests: read`, `id-token: write` | Needed for OIDC relay exchange. |
| Core write and future continuation | `contents: write`, `pull-requests: write` | Reserved for future trusted fix-push support using the GitHub App installation token. Current live redispatch is disabled until that support exists. |

No issue permission is included in this change. Terminal failures go to job summary and artifacts, not issue creation, PR comments, or labels.

## Trust Boundary

The trusted boundary is the `DongwonTTuna-Labs` organization plus the GitHub App and OIDC relay trust configured for this infrastructure. A job must validate PR identity, current head SHA, base ref, actor, repository owner, and fork status before relay setup, checkout of PR code, writes, or continuation dispatch.

Fork PRs are not write-capable in this contract. If a fork PR reaches the loop, it must terminate before relay setup or write operations with a clear terminal reason in the job summary and artifact.

Same-repo fix pushes are future work. The current reusable workflow validates trust and stage output, but if a live stage asks to redispatch it terminates with `trusted-fix-push-not-implemented` rather than reusing an unchanged PR head. A future fix-push change must keep the stale-SHA guard, push only trusted same-repo branches, emit an `updated_head_sha`, and dispatch that updated SHA with the GitHub App installation token.

## Loop Termination

The loop has four stages: `review`, `design`, `fix`, and `issue`.

A successful terminal run happens when Codex returns a machine-readable result that the PR is ready and no further design, fix, or issue stage is required.

A continuing run is future work. It may happen only after the `fix` stage pushes a trusted same-repo commit, emits `updated_head_sha`, and dispatches a new `stage=review` event with `iteration` increased by `1` and the updated `head_sha`. In the current implementation, `should_redispatch=true` is terminal with `trusted-fix-push-not-implemented`.

A failure terminal run happens when validation fails, max iteration is exceeded, SHA is stale, the actor isn't trusted, required GitHub App secrets are missing, the payload is malformed, Codex output is ambiguous, the PR is closed or missing, the PR comes from a fork, or a write fails.

Terminal failures must be visible through job summary and artifacts. Labels and comments MUST NOT be used for internal loop state, including terminal failure state.

## Rollout

Start with a SHA-pinned org-internal consumer on one trusted repository. Use `stage=review`, `iteration=0`, and a unique `correlation_id` for the first run. Keep the first consumer in `dry_run: true` until the workflow summaries and artifacts prove the loop can read PR context, validate trust, and terminate without writes.

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
| `<max-iterations>` | Optional limit, usually `1` for a smoke run. |

Confirm the PR identity before dispatching:

```bash
gh pr view <pr-number> \
  --repo <owner>/<repo> \
  --json number,state,headRefOid,baseRefName,headRepositoryOwner,headRepository
```

The PR should be open, same-repository, and dry-run only. The `headRefOid` must equal `<head-sha>`, and `baseRefName` must equal `<base-ref>`.

Dispatch payload schema smoke command:

```bash
gh api repos/:owner/:repo/dispatches \
  --method POST \
  --field event_type=codex-loop \
  --field client_payload[pr_number]=<pr-number> \
  --field client_payload[head_sha]=<head-sha> \
  --field client_payload[base_ref]=<base-ref> \
  --field client_payload[stage]=review \
  --field client_payload[iteration]=<iteration> \
  --field client_payload[correlation_id]=codex-loop-<pr-number>-<head-sha>-<iteration> \
  --field client_payload[requested_by]=<requested-by> \
  --field client_payload[dry_run]=true \
  --field client_payload[max_iterations]=<max-iterations>
```

The required `client_payload` keys are `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, and `requested_by`. Optional keys are `dry_run` and `max_iterations`. Keep `dry_run=true` for smoke tests.

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
3. The reusable core receives the same `pr_number`, `head_sha`, `base_ref`, `stage`, `iteration`, `correlation_id`, `requested_by`, `dry_run`, and `max_iterations` values.
4. The run remains dry-run and doesn't create dirty label/comment state.
5. Any failure is terminal in the job summary or artifacts, not represented by labels or comments.

Verify the first rollout by checking the reusable workflow inputs, the dispatch adapter payload validation, the OIDC relay exchange path, the GitHub App token path for write-capable jobs, and the absence of label/comment state changes.

Expand to more repositories only after the first consumer records clean summaries, artifacts, and loop termination behavior.

## Rollback

Start with the least destructive stop: disable or remove the consumer workflow call that invokes `DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@<sha-or-tag>`. This stops new consumer-triggered runs without deleting shared workflow source.

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

Rollback order:

1. Disable or remove the consumer workflow call first.
2. Disable or remove the `codex-loop-dispatch.yml` trigger or adapter if dispatch can still enter the loop.
3. Leave the reusable workflow source inert when there are no callers.
4. Cancel active runs tied to the affected `correlation_id`.
5. Rotate or suspend GitHub App credentials if unexpected write behavior is suspected.

## Non-Goals

This change doesn't package or describe external distribution.

This change doesn't allow PR labels or issue/PR comments as orchestration state. It MUST NOT use PR labels for internal loop state and MUST NOT use issue/PR comments for internal loop state.

This change doesn't add issue creation or issue permissions. The `issue` stage is a loop stage name only for now, and terminal failures stay in job summaries and artifacts.

This change doesn't add PAT fallback language or support.

This change doesn't make fork PRs write-capable.
