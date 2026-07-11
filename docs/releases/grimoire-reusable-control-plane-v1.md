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

Current Grimoire privileged GitHub operations use GitHub App installation-token auth as recorded in `docs/decisions/grimoire-app-auth.md` and `docs/releases/grimoire-app-auth-v1.md`. Consumers map `GRIMOIRE_APP_PRIVATE_KEY` and pass `grimoire_app_client_id`; the reusable workflow mints a short-lived App token and feeds checkout plus downstream GitHub mutation paths. Legacy PAT secrets, `CODEX_LOOP_PAT`, `GITHUB_TOKEN`, and `github.token` aren't valid privileged Grimoire auth sources. Model stages still use named `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET` first, then same-name runner environment fallbacks for the relay and Cloudflare Access values. The OpenCode provider sends `CF-Access-Client-Id` and `CF-Access-Client-Secret` as environment-backed request headers. All credential paths are masked and must not appear in docs, logs, fixtures, comments, or evidence.

## Migration Next

Consumer repositories should migrate to a thin caller workflow that calls `DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main`, passes PR metadata inputs, passes `grimoire_app_client_id`, maps `GRIMOIRE_APP_PRIVATE_KEY`, keeps relay and Cloudflare Access secret mappings, and includes guarded `pull_request.unlabeled` handling for `📋 Spec Needed` removals.

The first observed real cross-repo PR-event run belongs to the later rollout task after human merge, private access setup, and required secret setup.
