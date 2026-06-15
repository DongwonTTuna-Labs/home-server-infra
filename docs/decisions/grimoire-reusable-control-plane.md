# ADR: Grimoire Reusable Control Plane

## Status

Accepted for Task 1 documentation and contract-test scope. Runtime workflows, composite actions, config files, schemas, and live rollout are later tasks.

## Context

`home-server-infra` PR #53, commit `8ed807f6b6d3676b001164dc2116bf87f117d69b`, removed the old reusable Codex loop infrastructure from this repository. Grimoire is not a rollback of that Codex system. It is a relocation of the recovered Grimoire opencode and OMO PR loop from the `rs-builder-relayer-client` pilot into `home-server-infra` as private reusable control-plane infrastructure.

The pilot recovered a self-referential PR loop, not a generic CI package. The control plane reviews a pull request, designs scoped work, files out-of-scope issues, fixes only in-scope work when evidence is sufficient, verifies the result, and either terminates or pushes one scoped bot commit that triggers a fresh `synchronize` review.

## Source Of Truth

`home-server-infra` is the source of truth for Grimoire control-plane workflows, actions, stage helpers, config, schemas, tests, and docs. Consumer repositories must keep only a thin caller workflow and consumer-specific policy notes.

The Task 1 source material is:

1. `rs-builder-relayer-client/.omo/plans/grimoire-reusable-control-plane.md`
2. `rs-builder-relayer-client/.omo/drafts/grimoire-loop-history.md`
3. This ADR
4. `docs/grimoire-reusable.md`
5. `tests/grimoire_doc_contract_test.py`

## Recovered Stage Map

The recovered stage list is exact and ordered:

1. `trusted-controller`
2. `review`
3. `design`
4. `spec-gap`
5. `fix`
6. `verify`
7. `labels`
8. `cast`

The control flow is `trusted-controller -> review -> design -> file deduped out-of-scope Issues -> spec-gap or fix -> boulder -> verify -> terminate or loop`. `cast` is the driver that sequences the stages, runs the boulder continuation, applies label transitions, and decides terminal or re-review behavior from machine-readable artifacts.

`review` is read-only. `design` is the OpenSpec and OMO scope authority. `spec-gap` halts with a five-section comment when evidence is insufficient. `fix` may prepare and run Atlas work only inside the active scope. `verify` emits the F1-F4 verdict, and all four fields must approve before either terminal cast or one scoped bot push.

## Package Boundary

Task 1 defines the package boundary only. Task 2 owns runtime relocation and stage action creation. The target package map is:

| Area | Path |
| --- | --- |
| Reusable workflow | `.github/workflows/grimoire-control-plane.yml` |
| Trusted controller action | `actions/grimoire/trusted-controller/action.yml` |
| Review action | `actions/grimoire/review/action.yml` |
| Design action | `actions/grimoire/design/action.yml` |
| Spec-gap action | `actions/grimoire/spec-gap/action.yml` |
| Fix action | `actions/grimoire/fix/action.yml` |
| Verify action | `actions/grimoire/verify/action.yml` |
| Labels action | `actions/grimoire/labels/action.yml` |
| Cast driver action | `actions/grimoire/cast/action.yml` |
| Action-local helpers | `actions/grimoire/<stage>/scripts/*` |
| OpenCode config | `config/grimoire/opencode.json` |
| OMO config | `config/grimoire/oh-my-openagent.jsonc` |
| Workflow-call schema | `schemas/grimoire-workflow-call.v1.schema.json` |
| Workflow contract test | `tests/grimoire_workflow_contract_test.py` |
| Action contract test | `tests/grimoire_action_contract_test.py` |
| Stage contract test | `tests/grimoire_stage_contract_test.py` |
| Secret hygiene test | `tests/grimoire_secret_hygiene_test.py` |
| Doc contract test | `tests/grimoire_doc_contract_test.py` |
| Consumer adapter validator | `tests/validate_consumer_adapter.py` |
| Operator guide | `docs/grimoire-reusable.md` |
| Decision record | `docs/decisions/grimoire-reusable-control-plane.md` |

The `.sh` and `.py` helpers are implementation details under the owning action. There is no flat top-level `scripts/grimoire/` runtime API.

## Consumer Policy

Grimoire consumers are private repositories and must call the reusable workflow on `@main`:

```yaml
jobs:
  grimoire:
    uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main
```

The `@main` policy is intentional for this private control plane. Valid consumer docs and examples must not recommend SHA, tag, or branch pins other than `@main`. Non-`@main` examples may appear only when clearly labeled as invalid examples.

The called workflow needs repository access enabled in `home-server-infra` Settings, Actions, General, Access before private consumers can use it.

## Security And Auth

Grimoire uses PAT-only GitHub auth for privileged operations. The intended contract is `GRIMOIRE_PAT` as the named consumer secret, with the existing runner `CODEX_LOOP_PAT` fallback tension left as an explicit Task 3 and Task 4 contract point. Model-capable stages require explicit `AI_RELAY_API_KEY`, `CF_ACCESS_CLIENT_ID`, and `CF_ACCESS_CLIENT_SECRET` named secrets with same-name runner environment fallback for the relay and Cloudflare Access values; the OpenCode provider maps the CF values to `CF-Access-Client-Id` and `CF-Access-Client-Secret` headers. Grimoire must not recommend `GITHUB_TOKEN`, GitHub App tokens, `secrets: inherit`, `pull_request_target`, or GitHub-hosted runner fallback for privileged control-plane work.

The runner contract remains `Home Server Runners` with label `dongwontuna-labs-runner`. The reusable workflow keeps `permissions: {}` at the top level and grants only explicit job permissions if a later task proves they are needed.

The trusted controller runs from the base control-plane copy before model credentials or write credentials touch PR-head data. Consumer checkout is data. Control-plane checkout is explicit. PR-head `.opencode`, `opencode.json`, actions, scripts, and package lifecycle hooks are not trusted under credentials.

## Scope Guard

The scope authority is OpenSpec plus OMO. The priority order is:

1. Active `openspec/changes/*` artifacts when present
2. This Grimoire reusable-control-plane plan while the relocation is in progress
3. Explicit user follow-up decisions recorded in current planning notes

`design` binds review findings to that active scope and emits in-scope and out-of-scope classifications. Out-of-scope findings are filed as short GitHub Issues right after design, deduped by stable fingerprint, redacted, and limited to Issues-only mutation. They are not fixed inside the Grimoire loop.

## Runtime Policy

There is no runtime simulation input, no dry-run toggle, and no separate manual Grimoire workflow in the reusable control plane. Safety comes from the pull request event gate, the `grimoire:disabled` stop label, the trusted-controller protected-path guard, the fix scope guard, and the F1-F4 verification gate.

Local tests and fixtures prove behavior before rollout. Live cross-repo evidence is a later task after the infra PR is merged and private reusable workflow access is enabled by a maintainer.

## Consequences

This decision keeps `home-server-infra` as the private source of truth and keeps consumer repositories thin. It also separates documentation and contract boundaries from runtime relocation so Task 2 can create the stage actions without changing the contract.

The main risk is auth naming drift between `GRIMOIRE_PAT` and the existing `CODEX_LOOP_PAT` runner environment, plus consumer drift in the explicit relay and Cloudflare Access secret mapping. The workflow, schema, docs, and consumer adapter validator make those names explicit and fail closed when required model-capable credentials are absent.
