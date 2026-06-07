# Codex Loop (reusable core)

`codex-loop-reusable.yml` is the org-reusable Codex review/autofix loop. It is
**private-repo-only infra; security is intentionally out of scope.**

## Consumers pin `@main`

Consumer adapters call the core at the mutable `@main` ref, not a commit SHA:

```yaml
uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@main
```

So changes to the core take effect for all consumers on merge to `main` — no
per-consumer SHA re-pin.

## Required runner environment variables (no GitHub secrets)

Credentials are **environment variables on the self-hosted org runners**
(`stacks/codex-github-runners/`), not GitHub Actions secrets. The reusable core
declares no `secrets:`; consumer adapters map none. Every runner must export:

| Env var | Purpose |
| --- | --- |
| `CODEX_RELAY_API_KEY` | Static codex-lb dashboard key. A `run:` step reads it from the runner env, masks it, and feeds it to `openai/codex-action` as `openai-api-key` (relay endpoint `https://relay-ai.dongwontuna.net/v1/responses`). Replaces the old OIDC relay-token exchange. |
| `CODEX_LOOP_PAT` | Permanent classic PAT (repo scope), read directly in the push / continuation-dispatch `run:` steps. A PAT (not `GITHUB_TOKEN`) is required so `repository_dispatch` re-triggers the workflow. Replaces the old GitHub App. |

A job that lands on a runner missing either var fails fast (`:?` guard).

## Behavior

- **No security gates.** There is no trust/fork/stale guard; any trigger runs live.
- **Always live.** No dry-run. Eligible triggers make real model calls, post PR
  review comments, push autofix commits, and dispatch the next stage.
- **Unlimited until LGTM.** There is no iteration cap (`autofix.max_rounds` is set
  effectively unlimited in `setup/codex-review/config.yml`). The loop re-dispatches
  review → design → fix → push → review … until a review stage returns LGTM. The
  only remaining stop besides LGTM is the no-progress / oscillation guard in
  `loop/state.py` (the model repeating an identical no-op patch), which prevents a
  pointless infinite loop.

## Jobs

`validate → setup-state → run-stage → finalize`
- **validate** — typed-input sanity (stage ∈ review/design/fix/push, iteration).
- **setup-state** — checkout PR data, install the trusted `codex-review` helper
  (pinned `setup-codex-review` action), read/bootstrap loop state, upload it.
- **run-stage** — run the stage's model steps (static relay key) and, on the fix
  path, push the validated autofix (PAT).
- **finalize** — resolve LGTM / next stage and, when continuing, `repository_dispatch`
  the next iteration (PAT).
