# setup/codex-review

이 디렉터리는 Codex PR review v3 workflow의 실제 구현 단위가 들어있는 helper package다.

현재 상태는 실행 가능한 implementation이다. 공통 기반, GitHub helper, security policy, resolve-gate-reentry CLI, action-friendly prompt/schema helpers, trusted push orchestration, workflow guardrail test가 포함되어 있다. GitHub Actions model job은 pinned `openai/codex-action`이 prompt/schema/output 파일을 받아 실행하고, helper가 그 결과를 다시 stage별 validator로 검증한다.

## 패키지 레이아웃

`src/codex_review/` 는 책임별 레이어로 묶여 있다:

- `core/` — 공통 기반: `config`, `env`, `paths`, `artifacts`, `schema`, `logging`, `errors`, `output`, `constants`
- `model/` — `adapter`(model 실행), `inspection`(model 근거 검증)
- `patches/` — `commit_plan`, `fix_edits`
- `context/` — PR/diff/budget/docs/openspec/threads context builder
- `github/` — GitHub API client (`app_token`, `oidc_token`, `pull_requests`, `review_threads`, ...)
- `security/` — `provenance`, `redaction`, `patch_policy`, `permissions`, `checkout`, `subprocess_env`
- `loop/` — autofix loop state machine (`state`, `router`, `events`)
- `cli/` — area별 `handlers/` + `registry` + thin `main()` dispatcher
- `stages/` — resolve_gate~issue_fallback 단계 로직 (resolve gate → review → techlead → design → design chief → fix dispatch → merge → push → reentry → issue fallback)

## 구현/검증 범위

1. resolve_gate: resolve gate fixture/GitHub read/dry-run/actual apply path
2. review + techlead: artifact validation/publication path
3. design + design_chief: design routing/publication path
4. fix_dispatch + fix_merge + push: autofix patch dispatch/merge/push guard path
5. reentry: reentry record/validation path
6. workflow shape test로 라벨 구동 split workflow(`codex-review.yml`, `codex-design.yml`, `codex-fix.yml`, `codex-issue.yml`)를 강제

## 모델 실행 연결

workflow model stages는 `openai/codex-action`을 사용한다. 각 stage는 먼저 helper CLI로 prompt를 만들고, base schema를 OpenAI Structured Outputs strict schema로 변환해 `output-schema-file`에 넘긴다. action이 JSON artifact를 쓴 뒤에는 helper validator가 원래 stage schema와 stage별 business rule로 결과를 다시 검증한다.

로컬 테스트나 별도 consumer가 필요한 경우 provider-neutral `CODEX_REVIEW_MODEL_COMMAND` adapter는 CLI 하위호환 경로로 남아 있지만, repository workflow는 이 adapter나 runner script에 의존하지 않는다.

## 가장 중요한 원칙

- PR head의 `setup/` 코드는 절대 실행하지 않는다.
- trusted script는 base SHA checkout에서만 실행한다.
- model output은 항상 검증 후 trusted job에서만 side effect를 적용한다.
