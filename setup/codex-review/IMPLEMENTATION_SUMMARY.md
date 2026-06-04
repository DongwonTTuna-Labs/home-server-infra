# Codex Review v3 Implementation Summary

This package is no longer a spec-only skeleton. It now includes runnable helpers for the full Codex Review v3 flow:

- foundation: config, artifact IO, schema loading, paths, CLI, GitHub Actions outputs
- GitHub boundary: REST/GraphQL client, PR/review-thread/comment/issue/review helpers, markers, App-token helper
- security: provenance checks, checkout guards, secret redaction, patch policy, permissions checks
- context: diff parser, changed-line map, PR/review/docs/file inventory context builders
- loop: route decisions, loop state, audit events
- resolve_gate through reentry: resolve gate, review, techlead, design, design chief, fix dispatch, fix merge, trusted push, reentry recording
- model execution: prompt helpers, OpenAI strict schema generation, and validator commands for every pinned `openai/codex-action` output artifact
- workflow: four label-driven workflows (codex-review/design/fix/issue) with Codex Action model jobs separated from trusted write jobs, explicit route gates, and no inline schema/Python bloat
- tests: unit/workflow coverage for lifecycle, review validation, techlead, design, patch policy, dry-run publishing, fix collection, model adapter, push guards, routing, and event helpers

## Verification

Run from the repository root:

```bash
PYTHONPATH=setup/codex-review/src python3 -m compileall -q setup/codex-review/src/codex_review
PYTHONPATH=setup/codex-review/src pytest -q setup/codex-review/tests
```

The helper CLI is available through:

```bash
setup/codex-review/bin/codex-review --help
```

## External integrations

The GitHub API, Codex Action model path, and final branch push paths are implemented as guarded helpers. A real repository still needs the correct GitHub App credentials, relay binding, workflow secrets, and repository-specific test commands before trusted writes can succeed. Model jobs write OpenAI strict schema-constrained artifacts through `openai/codex-action`; trusted jobs validate those artifacts again before minting a GitHub App token for push or issue fallback.

## GitHub specification hardening applied

The split workflows and helpers have been hardened against the GitHub Actions/API issues called out in the final review:

- trusted write/push stages now obtain a GitHub App installation token through `codex-review auth app-token`
- actual resolve_gate/techlead/design_chief/push writes actively verify the token through the installation-token-only `/installation/repositories` endpoint
- push push no longer relies on `actions/checkout` persisted credentials; it sets an authenticated remote with the installation token only for the push and restores the original remote afterward
- every checkout step sets `persist-credentials: false`
- techlead review creation validates current PR head drift and sends `commit_id` for inline reviews
- deferred issue search uses `GITHUB_API_URL` and URL-encoded Search API queries
- `workflow_dispatch.inputs.pr_number` is threaded through event/context resolution via `CODEX_REVIEW_PR_NUMBER`
- design prompt/action model planning is followed by explicit `design validate-plan`
- record-reentry persists loop state through the GitHub App token only after an actual push
- first-party GitHub Actions are SHA-pinned across the split workflows, and shared job setup is factored into the `setup-codex-review` composite action
