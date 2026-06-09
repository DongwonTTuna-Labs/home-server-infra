# Codex Loop Phase 2 opencode and OMA Feasibility Spike

## Scope

This document is a feasibility spike for a later runtime migration from the current Codex Loop model runner to opencode and optional Oh My OpenAgent orchestration. It is not an implementation plan for Phase 1. Phase 1 keeps the existing runtime and only adds PR branch memory plus prompt context wiring.

This spike answers four questions.

1. Can the current `openai/codex-action@v1` model steps be represented with opencode headless runs?
2. Can CI run opencode without interactive permission prompts?
3. Can the repo keep the current artifact validators and schema contracts while changing only the model execution surface later?
4. Should OMA Team Mode be part of Phase 2, or should it remain behind a later decision gate?

## Current Repository Truth

The live reusable core is `.github/workflows/codex-loop-reusable.yml`. It currently uses `openai/codex-action@v1` for review, techlead, design, fix, merge, and semantic safety model work.

Current model step traits:

1. The action is called from the `run-stage` job after `pr-head` is checked out as data with `persist-credentials: false`.
2. The action receives `openai-api-key` from the runner environment through `CODEX_RELAY_API_KEY`.
3. The action sends requests to `https://relay-ai.dongwontuna.net/v1/responses`.
4. The action sets `effort: xhigh`, `safety-strategy: unsafe`, `sandbox: danger-full-access`, a fresh `codex-home`, and `working-directory: pr-head`.
5. Each model step writes a raw JSON artifact such as `findings.raw.json`, then trusted `codex-review` validators check the artifact against local schemas.
6. The push path uses `CODEX_LOOP_PAT` only after local validation, semantic safety, and remote head verification.

The practical migration target is therefore not a new workflow topology. It is a new model execution adapter that still reads trusted prompts, runs against `pr-head`, writes the same raw JSON files, and lets existing validators decide pass or fail.

## Non Goals

1. No Phase 1 runtime migration.
2. No workflow YAML change as part of this task.
3. No Python source or test change as part of this task.
4. No opencode, OMA, npm, Bun, or package installation as part of this task.
5. No Team Mode adoption yet.
6. No new vector DB, embeddings store, external memory service, issue state, label state, or comment state.
7. No relaxation of existing artifact validation, semantic safety, or PR head trust boundaries.

## Feasibility Findings

### opencode CLI Headless Run

OpenCode supports non-interactive command execution through:

```sh
opencode run --format json --dir pr-head --model ai-relay/gpt-5.5 --agent build "Read ../codex-review-artifacts/review/correctness/prompt.md and write schema-valid JSON."
```

`--format json` can stream raw JSON events. `--dir` can set the working directory to `pr-head`, matching the current action `working-directory: pr-head` behavior. The direct CLI path is the best first custom adapter candidate because it can be wrapped with shell redirection and post-run extraction before existing `codex-review ... validate` commands run.

Open question: the spike must prove the exact event shape from `--format json` and select one stable extraction rule for the assistant payload. That proof belongs in the future Phase 2 smoke, not in Phase 1.

### opencode Server and SDK Option

OpenCode also supports a headless server:

```sh
opencode serve --hostname 127.0.0.1 --port 4096
```

The server exposes HTTP APIs and an OpenAPI spec. The documented SDK path can connect to a running server, create or reuse sessions, call prompt APIs, and request structured output. This option may reduce cold boot cost across many sequential model steps, especially when MCP servers or agent config initialization become expensive.

The server and SDK path is viable only if Phase 2 adds a trusted wrapper that handles server lifetime, local-only binding, authentication through `OPENCODE_SERVER_PASSWORD`, cleanup, and deterministic mapping from API responses to the existing raw JSON artifacts.

### Official opencode GitHub Action

The official GitHub action is:

```yaml
uses: anomalyco/opencode/github@latest
```

It is useful as the first smoke because it exercises opencode on a runner with the least custom code. It accepts a required `model` in `provider/model` form, an optional `agent`, an optional `prompt`, and token choices for GitHub operations.

For this repo, the official action should be a smoke target first, not the final adapter. The current Codex Loop needs exact artifact names, schema files, local validators, and careful write boundaries. A custom `opencode run` wrapper is likely a closer fit than a comment-oriented GitHub agent action.

### OMA CLI Orchestration

Oh My OpenAgent has a non-interactive CLI path:

```sh
bunx oh-my-openagent run --json --agent <agent> "Run the Codex Loop review prompt and emit the expected schema."
```

This may become useful after the single-agent opencode path is stable. It can map Codex Loop stages to OMA agents or categories and may help preserve Prometheus, Atlas, and executor-style separation at the harness layer.

It should not be the first migration step. OMA introduces another orchestration layer, extra config files, and more moving pieces. Phase 2 should first prove a one-step opencode adapter with the existing schemas.

### Team Mode Status

Team Mode is experimental for this workflow. Current OMA docs describe it as opt-in and off by default, with team tools enabled only after configuration and restart. It can coordinate multiple members, shared tasks, and optional tmux views, but that power also changes the safety model.

Decision gate: Team Mode must stay disabled until the single-agent adapter is stable, schema output is deterministic, artifact validators pass, permission prompts are eliminated, and failure cleanup is proven. It is a later orchestration experiment, not a baseline Phase 2 dependency.

## Relay Compatibility

The local OMA config already maps multiple categories and agents to `ai-relay/gpt-5.5`. The current workflow calls the relay endpoint directly through `openai/codex-action@v1` and uses `effort: xhigh`.

Phase 2 must prove that opencode can resolve and call the same logical model name, `ai-relay/gpt-5.5`, with the intended reasoning variant or equivalent model option. If opencode uses provider model resolution through config rather than a direct responses endpoint flag, the runner must provide only non-secret config plus the existing relay key material through runner environment variables. Secret values must not be written to workflow YAML, docs, artifacts, prompts, or evidence.

Success requires parity at the artifact boundary, not byte-for-byte model output parity.

## CI Permission Model

CI cannot stop for approvals. Every candidate path must run with explicit allow and deny rules through `OPENCODE_PERMISSION`. The rule must have no `ask` outcome in CI.

A starter shape for a smoke-only read review could be:

```sh
export OPENCODE_PERMISSION='{"read":"allow","grep":"allow","glob":"allow","lsp":"allow","webfetch":"deny","websearch":"deny","edit":"deny","bash":{"*":"deny"},"question":"deny","task":"deny","external_directory":"deny"}'
```

A later fix-stage adapter would need a narrower write policy, not `edit: allow` for the full repository. It should permit writes only through a controlled output path or through the same existing `codex-review` patch and commit-plan validators. The preferred design is still: model proposes JSON, trusted code validates and applies.

CI safety requirements:

1. No PR-head script execution under privileged tokens.
2. No package install from PR-head.
3. No interactive permission prompts, no `ask` in `OPENCODE_PERMISSION`.
4. No blanket `--dangerously-skip-permissions` in the main loop.
5. Deny network tools unless a stage explicitly needs them and the trust model approves them.
6. Preserve current artifact validators as the final gate for each raw model output.
7. Keep `pr-head` as untrusted data and use trusted helper code from the reusable core source.

## Staged Migration Path

### Stage 0, Documentation and Local Decision

Keep Phase 1 unchanged. Finish memory and workflow docs. Decide which stage is safest for a smoke, likely review correctness because it is read-only and already writes one raw JSON artifact.

Gate to exit: this spike reviewed and no Phase 1 runtime files changed.

### Stage 1, Official Action Smoke

Create a separate non-required workflow or manual job that uses `anomalyco/opencode/github@latest` against a harmless prompt on the self-hosted runner. It should use `contents: read`, `pull-requests: read`, and no write token. The prompt should inspect static checked-out data and write a summary artifact only.

Gate to exit: action starts, authenticates through approved runner env, uses `ai-relay/gpt-5.5` or a documented fallback, and exits without permission prompts.

### Stage 2, Direct Single-Agent `opencode run` Review Adapter

Replace one review model call in an experimental branch with a shell wrapper around:

```sh
opencode run --format json --dir pr-head --model ai-relay/gpt-5.5 --agent build "..."
```

The wrapper must read the existing prompt file, capture raw event output, extract the final assistant JSON, write `codex-review-artifacts/review/correctness/findings.raw.json`, and then run the existing strict schema validator.

Gate to exit: `codex-review review validate` passes on valid outputs and fails closed on malformed outputs.

### Stage 3, Schema and Artifact Parity

Run each current model stage through the adapter in a non-writing branch: review, techlead, design inventory, design clusters, design plan, design chief, fix merge, and semantic safety. Keep push disabled until artifact parity is stable.

Gate to exit: all existing raw artifact files are produced at the same paths and consumed by the same validators.

### Stage 4, Controlled Fix Path Trial

Allow the model to propose fixes only as schema JSON. Trusted `codex-review` code still validates patches, applies commit plans, force-adds only trusted memory files, performs semantic safety checks, and pushes only after current remote head verification.

Gate to exit: no direct opencode edit reaches the repository without the current validators.

### Stage 5, Optional OMA Single-Agent Orchestration

Try:

```sh
bunx oh-my-openagent run --json --agent <agent> "..."
```

Use it as an orchestration wrapper only after Stage 2 and Stage 3 are stable. It must emit the same raw JSON artifact contracts and must not replace `loop-state.v1` routing or PR branch memory.

Gate to exit: OMA adds measurable value without weakening schema validation, permissions, or failure recovery.

### Stage 6, Team Mode Experiment

Evaluate Team Mode only in a separate experimental workflow. It must stay opt-in, non-required, and read-only until it proves deterministic shutdown, bounded runtime, bounded output, clean artifact collection, and safe failure handling.

Gate to exit: Team Mode has a documented pass/fail decision. Until then, production stays on single-agent orchestration.

## Blockers and Risks

1. JSON extraction risk: `opencode run --format json` may stream events, not final schema JSON. Phase 2 needs a tested parser.
2. Permission risk: any `ask` rule blocks CI. Any broad allow rule can bypass the current trust model.
3. Relay risk: `ai-relay/gpt-5.5` must resolve through opencode config on self-hosted runners without leaking secrets.
4. Artifact drift risk: opencode or OMA output may not match the current strict schemas without prompt and extraction changes.
5. Write-boundary risk: opencode agents can edit files. The loop should keep edits as model JSON proposals until trusted validators apply them.
6. Runner state risk: persistent self-hosted runners can retain config, sessions, caches, and server state. Smokes need cleanup checks.
7. Official action fit risk: `anomalyco/opencode/github@latest` is built for GitHub issue and PR interactions. The Codex Loop needs exact local artifacts, so the official action may stay smoke-only.
8. Team Mode risk: parallel agents add concurrency, shutdown, mailbox, and task-state failure modes. Treat it as experimental.
9. Supply-chain risk: `@latest` and `bunx` are moving inputs. Production would need pinning, cache policy, and update review.
10. Security posture risk: the live workflow is private infra and already uses powerful runner settings. Migration must not make PR-head code execution easier.

## Self-Hosted Runner Smoke Checklist

Use this checklist in a future Phase 2 branch. Do not run it during Phase 1.

1. Install and cache

   Confirm opencode is installed or cached from a pinned source. Confirm no install step runs from PR-head. Confirm `bunx oh-my-openagent` is either not used yet or pinned through an approved package policy.

2. Auth and relay environment

   Confirm `CODEX_RELAY_API_KEY` exists only in runner environment. Confirm opencode can resolve `ai-relay/gpt-5.5`. Confirm no secret value appears in logs, summaries, prompts, artifacts, or config committed to the repo.

3. Permission no-ask mode

   Export `OPENCODE_PERMISSION` with explicit allowlist and deny rules. Confirm there is no `ask`. Confirm a denied operation fails with a clear nonzero result rather than hanging.

4. Simple prompt

   Run a read-only prompt that lists current repository files or summarizes a checked-in markdown file. Capture JSON events from `opencode run --format json --dir pr-head`.

5. Schema output

   Ask for one existing schema shape, such as `review-axis-findings.v1`. Extract the final JSON and write the expected raw artifact path.

6. Artifact validation

   Run the existing `codex-review` validator for that artifact. Keep validation as the pass/fail signal.

7. Failure-mode tests

   Test malformed JSON, missing final answer, permission denial, relay auth failure, model timeout, killed process, and stale output file. Every failure must stop safely and preserve evidence.

8. Server path, if selected

   If testing `opencode serve`, bind to `127.0.0.1`, set `OPENCODE_SERVER_PASSWORD`, check `/global/health`, drive one prompt through the SDK or HTTP API, then stop the server and verify no process remains.

9. Git and workspace cleanliness

   Confirm only expected artifacts changed. Confirm `pr-head` was not used to install helpers or run package scripts.

10. Official action smoke

    Run `anomalyco/opencode/github@latest` in a non-required manual workflow with read-only permissions first. Do not grant write permissions until a separate decision approves it.

## Rollback Criteria

Rollback or stop the migration if any of these happen:

1. A model step can write repository files without trusted `codex-review` validation.
2. CI waits for a permission prompt.
3. `ai-relay/gpt-5.5` cannot be resolved or the relay path needs new secret handling not approved by infra owners.
4. Raw artifacts stop matching existing schemas.
5. Self-hosted runners retain unsafe server state, sessions, credentials, or modified config after a run.
6. Official action or OMA wrappers require broader GitHub permissions than the current stage needs.
7. Team Mode creates nondeterministic output, incomplete shutdown, hidden worktrees, or unbounded runtime.

## Success Metrics

1. One read-only review stage runs through opencode and produces the same raw artifact path.
2. Existing validators pass without changing schemas.
3. CI completes with no interactive prompts.
4. The runner uses `ai-relay/gpt-5.5` without exposing secrets.
5. Failure modes produce clear evidence and no partial trusted writes.
6. The custom adapter keeps or improves runtime compared with `openai/codex-action@v1` after cache warmup.
7. The migration removes no existing safety checks.

## Decision Summary

Phase 2 is feasible, but only as a staged migration. The best path is official action smoke, then a direct `opencode run` single-agent review adapter, then schema parity across the remaining stages, then optional OMA orchestration. Team Mode should remain behind a separate experimental gate.

Phase 1 does not implement runtime migration. It keeps `openai/codex-action@v1` and records this spike so the later opencode and OMA work starts from current repo truth instead of stale assumptions.
