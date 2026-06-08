# Codex Loop Reusable Core

`codex-loop-reusable.yml` is the org-reusable Codex review and autofix loop for this private infrastructure repository.

The current implementation is the runner-env model. It is not the earlier GitHub App, OIDC relay-token, split-stage, or PAT-forbidden model.

## Consumer Pinning

Current consumers call the reusable core from this repository, commonly at `@main`:

```yaml
uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@main
```

If a consumer needs stricter change control, pin to a reviewed SHA or internal tag. Either way, the consumer must follow the branch-protection and memory no-op guidance below.

## Required Runner Environment Variables

Credentials are environment variables on the self-hosted org runners under `stacks/codex-github-runners/`. They are not `workflow_call` secrets. The reusable core declares no `secrets:` and has no `${{ secrets.* }}` references.

| Env var | Purpose |
| --- | --- |
| `CODEX_RELAY_API_KEY` | Static codex-lb dashboard key. The workflow reads it from the runner env, masks it, and passes it to `openai/codex-action@v1` as `openai-api-key` for `https://relay-ai.dongwontuna.net/v1/responses`. |
| `CODEX_LOOP_PAT` | Permanent PAT used by trusted push and continuation-dispatch steps. The PAT is current behavior because `GITHUB_TOKEN` writes usually do not trigger follow-on workflows, while the loop needs memory and fix commits to re-trigger consumers. |

A job that lands on a runner missing the needed env var fails fast through shell `:?` guards.

## Jobs

The reusable core currently has six jobs:

```text
validate -> setup-state -> classify -> run-stage -> finalize -> required-checks
```

Core jobs are `validate`, `setup-state`, `run-stage`, and `finalize`. Control jobs are `classify` and `required-checks`.

| Job | Behavior |
| --- | --- |
| `validate` | Validates typed workflow inputs. |
| `setup-state` | Checks out base and PR-head data with `persist-credentials: false`, installs `codex-review`, bootstraps run-local loop state, and uploads the state artifact. |
| `classify` | Detects guarded memory-only commits before model work. |
| `run-stage` | Runs live review, design, fix, and push steps sequentially inside one job, with later steps gated by earlier stage outputs. |
| `finalize` | Resolves final outputs, handles `memory_only_noop`, and dispatches the next iteration only after a remote-verified push. |
| `required-checks` | Aggregates all prior jobs and fails only failed or cancelled dependencies. Skips are acceptable. |

There is no workflow input named `stage`. There are no cross-run state pointer inputs named `state_run_id` or `state_artifact_name`. `repository_dispatch` starts only the next full iteration after push verification.

## Memory Layer

Review memory lives under `.omo/review-memory/pr-<n>/` on the PR branch. The canonical files are `ledger.json`, `learnings.md`, `decisions.md`, `issues.md`, and `problems.md`.

This memory is advisory PR-scoped context. Treat it as untrusted PR data unless a specific entry is trusted by provenance. It may appear in prompt context, but it must not route stages, force LGTM, alter stale-head decisions, authorize writes, redispatch the loop, or suppress current findings through prompt text.

The workflow builds memory context once:

```bash
codex-review context memory \
  --pr-context codex-review-artifacts/common/pr-context.json \
  --repo-path pr-head \
  --out codex-review-artifacts/common/memory-context.md
```

The same `memory-context.md` is passed to the nine current prompt-builder calls.

## Memory-Only No-Op Gate

The no-op gate is job-level, not trigger-level:

1. `classify` runs before `run-stage`.
2. The classifier accepts a no-op only when all changed paths are review-memory paths, the head commit has `codex-memory: true`, and the actor/requester guard matches configured own actors.
3. `run-stage` is skipped when `needs.classify.outputs.should_run_model == 'false'`.
4. `finalize` emits `terminal_reason=memory_only_noop`, `should_redispatch=false`, and no updated head SHA.
5. `required-checks` stays green because skipped model work is allowed.

Do not add workflow-level `paths-ignore` to solve memory-only re-entry. It does not cover `repository_dispatch` or `workflow_call`, and trigger-level skips can strand required checks.

## Branch Protection

Consumer branch protection should require only this job:

```text
required-checks
```

Do not require `run-stage` or individual model jobs. They may legitimately skip on memory-only no-op commits.

## Loop Guards

Current guards are:

| Guard | Implementation |
| --- | --- |
| Actor guard | `classify` uses `github.actor`, `inputs.requested_by`, and `CODEX_LOOP_OWN_ACTORS`, defaulting to `github-actions[bot]`. |
| Memory marker | No-op requires a `codex-memory: true` commit marker. |
| Concurrency | `concurrency.group` is `codex-loop-${{ inputs.correlation_id }}`. Pass a PR-stable correlation id from consumers. |
| Aggregator | `required-checks` is the only branch-protection check consumers should require. |
| Push verification | Continuation dispatch needs a pushed and verified updated head SHA. |

`[skip ci]` and `skip-checks` do not stop `pull_request_target` or `repository_dispatch`. They only apply to `push` and `pull_request` workflows, so they are not a Codex Loop guard.

## Token Guidance

`GITHUB_TOKEN` is fine for read-only GitHub context where the workflow already uses `github.token`. It is not the current token for push or continuation dispatch.

Use `CODEX_LOOP_PAT` from the runner env for loop writes that must re-trigger follow-on workflows. Do not repeat the old PAT-forbidden fallback claim; the PAT is current implementation.

## PR-Scoped Non-Merge Guard

`.omo/` remains ignored on base branches. Trusted memory writers force-add canonical `.omo/review-memory/pr-<n>/` files on PR branches only. Memory should not land on `main` through normal merge.

Use this guard to flag review memory on base:

```bash
codex-review memory assert-not-on-base --base-ref main --repo-path <repo-path>
```

Clean base returns success. A base branch containing `.omo/review-memory/pr-<n>/...` returns non-zero.

## Same-Repo And Fork Safety

Privileged write-back must not execute untrusted PR-head code under base secrets or write tokens. In privileged consumer workflows, treat PR-head files as data. Do not run PR-head scripts, package-manager lifecycle hooks, composite actions, or helper installation while secrets are available.

For trusted same-repo writes, checkout or push the PR head explicitly, use the runner PAT only in the write step, and force-add only trusted generated memory files or validated fix outputs.
