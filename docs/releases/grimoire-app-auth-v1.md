# Release Notes: Grimoire App Auth v1

## Scope

This is the v1 auth update for the private reusable Grimoire control plane. It records the move from legacy PAT guidance to organization GitHub App installation-token auth for privileged GitHub operations.

This note covers documentation and contract state only. It doesn't claim production rollout, org-wide secret rollout, or observed cross-repo PR-event execution.

## Changed

1. The reusable workflow mints one short-lived Grimoire GitHub App installation token with `actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349`.
2. Token minting uses `client-id: ${{ inputs.grimoire_app_client_id }}`, `private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}`, and `owner: DongwonTTuna-Labs`.
3. Trusted control-plane checkout, consumer checkout, advisory comment upsert, Issues, labels, and scoped push use the minted App token path.
4. The downstream env name `GRIMOIRE_GITHUB_PAT` remains compatibility plumbing for existing helpers, but the value must come only from the minted App token output.
5. Legacy PAT secrets, `CODEX_LOOP_PAT`, `GITHUB_TOKEN`, `github.token`, and `secrets: inherit` aren't valid privileged Grimoire auth sources.
6. The consumer caller includes `pull_request.unlabeled` only for the guarded `📋 Spec Needed` removal path.

## Migration Checklist

1. Add or switch the consumer secret mapping to `GRIMOIRE_APP_PRIVATE_KEY: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}`.
2. Pass `grimoire_app_client_id` in the workflow call. The current default client ID is `Iv23liFL1dDHmU06FLSF`.
3. Keep explicit relay and Cloudflare Access secret mappings for `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET`.
4. Remove any privileged Grimoire dependency on `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GITHUB_TOKEN`, or `github.token`.
5. Add `pull_request.unlabeled` to the caller trigger list.
6. Keep the job guard: `(github.event.action != 'unlabeled' || github.event.label.name == '📋 Spec Needed')`.
7. Keep `permissions: {}` at the caller top level and keep the reusable workflow ref on `@main`.

## Spec Needed Re-Review UX

When OpenSpec evidence is missing and the case is advisory, cast posts or upserts the marker-backed `<!-- grimoire-spec-gap -->` advisory comment. The comment is neutral guidance and doesn't authorize code changes, commits, or push.

The labels action adds `📋 Spec Needed` as display-only advisory state. The owner fixes or adds truthful OpenSpec evidence, then removes `📋 Spec Needed`. The guarded `pull_request.unlabeled` event is the only label-clear re-review path.

## Security Notes

1. App installation tokens are short-lived and minted inside the trusted reusable workflow.
2. The App installation can be scoped to the repositories Grimoire is allowed to touch.
3. The token action is SHA-pinned and scoped to `owner: DongwonTTuna-Labs`.
4. If the private key is absent or token minting fails, Grimoire fails closed before checkout or mutation stages.
5. This repository has the current repo-level `GRIMOIRE_APP_PRIVATE_KEY` secret path. Organization-level secret and variable rollout requires an organization admin, and this release note doesn't claim that rollout is complete.
6. Docs, logs, fixtures, comments, and evidence must never include raw private keys, token-shaped literals, secret prefixes, secret lengths, secret hashes, token-bearing URLs, or private run URLs.

## Validation Evidence

1. Task 7 evidence records the consumer caller shape, guarded `pull_request.unlabeled` handling, explicit `GRIMOIRE_APP_PRIVATE_KEY` mapping, and `grimoire_app_client_id` input.
2. Task 8 evidence records one SHA-pinned token-mint step, App-token checkout wiring, downstream App-token plumbing, no live legacy PAT fallback, and secret-hygiene tests.
3. Task 9 evidence records static doc assertions and secret scans in `.omo/evidence/grimoire-app-task-9-docs.txt`.

## Limitations And Follow-Ups

1. No production rollout claim.
2. No observed cross-repo PR-event execution claim.
3. No claim that organization-level secret or variable rollout is complete.
4. T10 owns doc contract tests for this documentation update.
5. T11 owns later smoke, commit, and push after documentation and contract tests are complete.
