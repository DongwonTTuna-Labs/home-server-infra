# Release Notes: Grimoire Reusable Control Plane v1 Advisory Failure Separation

## Scope

This is a v1 minor behavior change for the private reusable Grimoire control plane. It separates advisory OpenSpec guidance from real safety failures.

No consumer workflow action is required. Existing private consumers keep the thin caller workflow, `@main` reusable workflow ref, explicit named secrets, and self-hosted runner policy.

## Changed

1. `success`, `neutral`, and `failure` are now the documented outcome classes.
2. `spec-gap-halt` keeps the compatible decision string, but cast reports `conclusion="neutral"`, exit 0, `status="ok"`, no push, and label intent `spec-needed`.
3. Spec-gap guidance is advisory for the current PR event and records `no_code_or_push_action=true`. It doesn't authorize code changes, commits, or push.
4. `run_complete` maps `neutral` to final `status="advisory"` and exit 0. Real failures still map to `status="fizzled"` and exit 1.
5. The labels action manages `📋 Spec Needed` as display-only advisory state. It removes managed running, done, and fizzled labels, adds `📋 Spec Needed`, and preserves unrelated labels.
6. `.omo/grimoire/scope.yml` is an optional v1 manifest. Valid `governed_paths` and `advisory_only_paths` use safe relative globs only. The manifest can narrow governed path evaluation, but can't expand write authority or bypass protected-path, unsafe-target, or scope guards.
7. Absent, malformed, or invalid scope manifests fall back to severity-threshold behavior.
8. The cast action exposes `conclusion` and `summary` outputs for optional presentation. This release doesn't add a separate neutral check-run.

## Still Fails Red

1. Protected path changes.
2. Unsafe target paths.
3. Malformed severities or malformed verdicts.
4. Scope violations.
5. Missing credentials.
6. Rejected F1-F4 verification.
7. Boulder liveness failures.
8. Push failure or unexpected push count.
9. High or critical missing OpenSpec evidence under severity-threshold or governed scope.

## Migration

No consumer action is required for the workflow call. Keep using `DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main`, keep `permissions: {}`, and keep explicit secret mapping for `GRIMOIRE_PAT`, `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET`.

To satisfy a spec-gap advisory, add or update the relevant OpenSpec evidence, keep the spec truthful, and let the PR rerun through normal `pull_request` events such as `synchronize`.

Consumers may add `.omo/grimoire/scope.yml` later if they need governed and advisory-only path families. The manifest is optional. Invalid manifests don't grant authority and fall back to severity-threshold behavior.

## Rollback

Rollback is reverting the cast_driver/labels/action/docs commits that introduced the advisory/failure separation. Do not change consumer auth, trigger policy, runner policy, or reusable workflow refs as a rollback path.

## Not Included

1. No production or live rollout claim.
2. No public marketplace action surface.
3. No manual dispatch or runtime control input.
4. No separate neutral check-run.
5. No removal of OpenSpec expectations.
