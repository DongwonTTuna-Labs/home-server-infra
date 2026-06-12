# Release Notes: Grimoire Reusable Control Plane v1

## Scope

This is the first reusable Grimoire control-plane package for private consumers. It relocates the recovered Grimoire opencode and OMO PR loop into `home-server-infra` as the source of truth.

This is not a Codex rollback. `home-server-infra` PR #53, commit `8ed807f6b6d3676b001164dc2116bf87f117d69b`, removed the old Codex loop. The new package keeps Grimoire's recovered stage order, scope guard, F1-F4 verification gate, and scoped push semantics.

## Included

1. Reusable workflow at `.github/workflows/grimoire-control-plane.yml` with `workflow_call` inputs for consumer repository, PR metadata, and optional contract version.
2. Eight composite stage actions under `actions/grimoire/`.
3. Controller-owned OpenCode and OMO config under `config/grimoire/`.
4. Schema, workflow, action, stage, secret hygiene, doc, and consumer adapter tests.
5. Local deterministic fixtures for `clear-noop`, `fixed-then-clear`, `spec-insufficient`, `reject`, and `boulder-incomplete`.
6. Secret hygiene tests that prove positive scans and negative leak fixtures without printing secret values.

## Not Included

1. No runtime simulation input.
2. No separate manual Grimoire workflow.
3. No manual dispatch or runtime control surface.
4. No GitHub-hosted runner fallback.
5. No `secrets: inherit` consumer model.
6. No production or live rollout claim.

## Access Gate

Private consumers can call the reusable workflow only after a maintainer enables access in `home-server-infra` under Settings, Actions, General, Access. The caller repository Actions policy must also allow private reusable workflows and actions from the organization.

## Auth Model

Privileged GitHub operations use PAT-only auth. The preferred named secret is `GRIMOIRE_PAT`; the self-hosted runner `CODEX_LOOP_PAT` fallback remains available and fail-closed. Model stages use named `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET` first, then same-name runner environment fallbacks for the relay and Cloudflare Access values. The OpenCode provider sends `CF-Access-Client-Id` and `CF-Access-Client-Secret` as environment-backed request headers. All credential paths are masked and must not appear in docs, logs, fixtures, comments, or evidence.

## Migration Next

Consumer repositories should migrate to a thin caller workflow that calls `DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main`, passes PR metadata inputs, and maps only `GRIMOIRE_PAT`, `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET` by name.

The first observed real cross-repo PR-event run belongs to the later rollout task after human merge and private access setup.
