# ADR: Grimoire GitHub App Auth

## Status

Accepted for Grimoire privileged GitHub operations. This records the docs and contract state after the App-token workflow, consumer trigger, and token-hygiene tasks. It doesn't claim production rollout, org-wide secret rollout, or observed cross-repo PR-event execution.

## Context

Grimoire needs privileged GitHub access for trusted control-plane checkout, consumer checkout, advisory comment upsert, out-of-scope Issue filing, display label updates, and one scoped bot push after F1-F4 approval.

The old documentation described PAT auth as the current model and didn't treat App-token auth as the valid privileged path. That no longer matches the reusable workflow. The workflow now mints one Grimoire GitHub App installation token with `actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349`, `client-id: ${{ inputs.grimoire_app_client_id }}`, `private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}`, and `owner: DongwonTTuna-Labs`.

Both checkouts use the minted App token. The cast stage receives that same minted token through the existing downstream `GRIMOIRE_GITHUB_PAT` environment name. That name is compatibility plumbing only. It is not a live PAT fallback.

## Decision

Grimoire uses the organization GitHub App installation token as the valid current auth path for privileged GitHub operations.

Consumer callers must pass `grimoire_app_client_id` and explicitly map `GRIMOIRE_APP_PRIVATE_KEY`. The reusable workflow owns token minting. Consumers must not provide PATs, `GITHUB_TOKEN`, `secrets: inherit`, or runtime auth controls for privileged Grimoire work.

The current caller trigger includes `pull_request.unlabeled` only for the `📋 Spec Needed` label removal path. The job guard must keep unlabeled reruns limited to `github.event.label.name == '📋 Spec Needed'`.

## Alternatives Considered

1. Keep PAT auth through a consumer secret. Rejected because a PAT is long-lived, harder to scope to Grimoire's exact repository set, easier to reuse outside the control plane, and weak for cross-repo privileged operations that need clear ownership.
2. Use `GITHUB_TOKEN`. Rejected because it is tied to the caller workflow context and doesn't provide the cross-repo permissions Grimoire needs for trusted control-plane checkout, consumer mutation, labels, Issues, comments, and scoped push.
3. Create one GitHub App per consumer repository. Rejected because it would multiply keys, installation policy, and rotation work across consumers. It would also make re-review behavior and contract tests drift by repository.
4. Use one organization-owned Grimoire GitHub App. Accepted because it gives least-privilege installation policy, short-lived installation tokens, one auditable owner, stable reusable workflow token minting, and the permissions needed for cross-repo control-plane operations and label-clear re-review.

## Consequences

Consumer setup changes from a PAT secret to a GitHub App private key secret plus a non-secret client-id input.

The reusable workflow still exposes the downstream `GRIMOIRE_GITHUB_PAT` environment name to existing stage helpers. That environment value must only come from the minted App token output. It must not come from a PAT, `CODEX_LOOP_PAT`, `GITHUB_TOKEN`, or `github.token`.

Token failures fail closed before checkout, comment, Issue, label, or push stages. Relay and Cloudflare Access auth stay separate from GitHub auth.

## Security Rationale

A GitHub App installation token is short-lived and minted inside the trusted reusable workflow. It narrows credential lifetime and avoids storing a reusable PAT for privileged Grimoire work.

The App installation can be scoped to the repositories Grimoire is allowed to touch. That matches the private reusable control-plane boundary better than personal credentials or caller-scoped `GITHUB_TOKEN` permissions.

The token action is SHA-pinned. The workflow passes `owner: DongwonTTuna-Labs`, so token minting is tied to the organization installation instead of caller-provided repository data.

Docs, logs, fixtures, comments, release notes, and evidence must never include raw private keys, token-shaped literals, secret prefixes, secret lengths, secret hashes, token-bearing URLs, or private run URLs.

## Migration And Operational Notes

Consumers must update their thin caller workflow to pass `grimoire_app_client_id` and map `GRIMOIRE_APP_PRIVATE_KEY` explicitly.

Consumers must add the guarded `pull_request.unlabeled` trigger if they want label-clear re-review after a `📋 Spec Needed` advisory. The guard must allow unlabeled reruns only when the removed label is `📋 Spec Needed`.

The `📋 Spec Needed` UX is advisory. Cast upserts the literal `<!-- grimoire-spec-gap -->` marker-backed advisory comment when spec evidence is missing, labels the PR for human attention, and performs no code action or push. The owner fixes or adds truthful OpenSpec evidence. Removing `📋 Spec Needed` then triggers re-review through the guarded `unlabeled` event.

Organization-level secret and variable rollout requires an organization admin. This repository has the current repo-level `GRIMOIRE_APP_PRIVATE_KEY` secret path, but these docs don't claim org-level rollout has happened.

## Validation

Task 7 evidence records the guarded `pull_request.unlabeled` caller shape, explicit `GRIMOIRE_APP_PRIVATE_KEY` mapping, `grimoire_app_client_id`, and absence of legacy PAT auth in the recommended snippet.

Task 8 evidence records one SHA-pinned token-mint step, App-token checkout wiring, App-token downstream plumbing through the stable env name, no live legacy PAT fallback, and secret-hygiene scans.

Task 9 doc verification records static assertions and secret scans in `.omo/evidence/grimoire-app-task-9-docs.txt` without secrets.
