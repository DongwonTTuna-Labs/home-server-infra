# AGENTS.md

이 저장소에서는 한국어로 소통한다. 정확성, 현재 증거, root-cause fix,
검증 가능한 evidence를 겉보기 완료보다 우선한다.

> **v2 안내 (2026-07-29).** ChatGPT Pro 웹 위임의 현재 운영 스택은 `stacks/gpt-webai-pro`
> (TypeScript 슬롯 데몬 + SQLite, Pro 전용). 진입점은 `gptpro` = `gpt-webai-pro run`이며
> 운영/설계는 `stacks/gpt-webai-pro/README.md`·`DESIGN.md`와 `~/.codex/runbooks/gpt-webai-pro.md`를
> 따른다. **아래 "PR #72 / Live·Pro / Release·Recovery" 규칙과 `stacks/gpt-webai-slot-pool`(v1,
> `gpt-webai-lifecycle`/`gptxhigh`)은 retired-in-place** — 코드는 이력으로 보존하되 새 GPT 위임·운영에
> 사용하지 않는다. cohort(slot-01..10), broker-attachments, auth-seed, `preflight --docker-slot-provider`,
> `show/resume --kind`, screenshot 기반 model-picker 게이트는 v1 전용이며 v2에는 없다.

## 최상위 불변 조건

- PR은 절대 직접 merge하지 않는다.
- 충분히 검증된 intended commit만 push한다. 중간/부분/미검증 작업은 push하지 않는다.
- unrelated user changes를 보존한다. 요청 없이 revert/reset/checkout 하지 않는다.
- secrets, cookies, browser profiles, auth-seed, tokens, private session state는
  zip, prompt, log, evidence, commit에 넣지 않는다.
- 테스트를 약화, 삭제, skip, fake해서 통과시키지 않는다.
- 이상 징후가 보이면 먼저 분류한다: implementation bug, test bug, spec mismatch,
  environment/provider issue, flaky timing, stale context, invalid assumption.

## PR #72 현재 Truth

- 컨텍스트 압축/재개 후 가장 먼저 아래 파일을 읽는다.
  1. `/home/dongwonttuna/.codex/attachments/28c679b5-4ae3-41a7-bfd9-0bd44f5d202b/goal.md`
  2. `/home/dongwonttuna/.codex/attachments/28c679b5-4ae3-41a7-bfd9-0bd44f5d202b/pr72-handoff.md`
  3. 현재 worktree, runtime state, evidence dirs
- 충돌하면 `goal.md`, handoff, 현재 증거를 우선한다.
- 이 대화 기억, 오래된 계획, 실패한 시도, retired notes보다 위 handoff와 현재 파일/런타임 상태를 우선한다.
- active goal이 UI에서 사라진 것처럼 보여도 먼저 goal tool 상태를 확인한다. goal objective가 위 handoff 파일을 참조하면 그 계약을 이어간다.
- PR #72의 현재 방향은 Rust supervisor/lifecycle + Node.js official Playwright/CDP provider 전환이다.
- 오래된 Node-only, Bash-heavy, Rust-excluded 메모는 stale context다. Rust+Node 방침을 임의로 축소하지 않는다.
- 이 전환은 점진 rollout이 아니라 PR #72 안에서 production send/poll/resume/show/download/release/status/cleanup 경로를 한 번에 Rust+Node로 넘기는 one-shot cutover다. 내부 구현/검증 단위는 rollback 가능한 evidence 수집과 위험 통제를 위한 것일 뿐이며, 일부 production Bash/agbrowse 의존을 다음 PR/future work로 남기지 않는다.

## PR #72 설계-구현-리뷰 역할 계약

- GPT Pro는 설계자이자 독립 설계/구현 리뷰어다. Codex는 실장/구현 책임자이자 프로젝트 관리자와 시니어 QA다.
- 모든 substantive behavior 변경은 `Pro design -> independent Pro design review LGTM/no-blocking -> Codex implementation and local verification -> independent Pro implementation review LGTM/no-blocking` 순서를 통과한다.
- 설계 리뷰가 `CHANGES_REQUIRED`이면 fresh designer Pro가 실패 행에 대한 decision-complete `DESIGN_DELTA_V1`을 반환한다. Codex가 anchor/hash/완전한 replacement/cross-document closure를 검증해 `.omo/plans/pr72-canonical-design/`의 canonical 문서에 직접 반영하고 manifest를 재생성한 뒤, 별도 Pro가 canonical 전체를 다시 리뷰한다. 9개 행 PASS와 정확한 `VERDICT: LGTM_NO_BLOCKING` 한 번까지 반복하며 승인 전 substantive 구현을 시작하지 않는다.
- 설계 수정 revision마다 ZIP 생성·다운로드를 요구하지 않는다. 승인된 최종 canonical 설계만 한 번 packaging하고 실제 Playwright download event 및 byte identity로 검증한다.
- Codex는 승인된 설계에 따라 source/test/runtime/script/docs를 직접 작성, 수정, 리팩터링하고 root-cause 진단, diagnostics, tests, smoke/live QA, evidence, commit/push를 수행한다.
- 구현 리뷰가 `CHANGES_REQUIRED`이면 Codex가 승인 설계 안에서 수정하고 관련 전체 검증 후 새 Pro 세션으로 재리뷰한다. 설계 변경이 필요하면 설계 게이트로 돌아간다. 동일한 최종 source/diff에 정확한 `VERDICT: LGTM_NO_BLOCKING` 한 번이면 통과하며 10회 LGTM을 요구하지 않는다.
- Pro coding source-tree artifact와 Codex 무편집 import는 더 이상 필수 workflow가 아니다. 기존 Manager-Only, artifact-only, Codex no-edit/quarantine 규칙은 폐기한다. 과거 artifact/import 기록은 provenance와 historical evidence로만 보존한다.
- PR #72에서 subagent는 읽기 전용 review/교차검증만 수행한다. canonical 설계 문서와 제품 source/test/runtime/script/docs는 Codex가 직접 수정·구현한다.
- Pro verdict, 설명, diff, 파일명, checksum은 그대로 신뢰하지 않는다. current source/diff, tests, logs, screenshot, DOM/CDP, runtime state와 대조한다.
- auth/authz, 권한, secret, DB/migration, 파괴·비가역 변경, public API, 결제, 외부 provider, 법률/compliance, 요청 architecture가 승인 설계로 명확하지 않으면 Codex가 추측하지 않고 설계 게이트로 돌아간다.

## PR #72 Cybersecurity 범위 제외

- 2026-07-14 사용자 지시에 따라 Pro 설계·리뷰·artifact·QA에서 cybersecurity 전용 공격/우회/취약점 재현 및 저수준 보안 샌드박스 과제를 제외한다.
- 과거 R10C12/R10C13의 보안 중심 요청과 산출물은 provenance로만 보존하며 재전송하거나 current design authority로 사용하지 않는다.
- 현재 설계 범위는 UI/model picker, attachment/send, session poll/resume/show, artifact download, slot/cohort fairness, durable state/recovery, CLI/provider envelope, release/runtime ownership, Git scope/docs/QA의 기능적 lifecycle 계약이다.
- secret/auth/profile 보호, access-control 우회 금지, 안전한 archive 처리, 테스트 무결성, unrelated 변경 보존, destructive action 금지, PR no-merge 불변조건은 그대로 유지한다.

## 구조 원칙

- Rust는 lifecycle/supervisor/state machine/session records/locks/slots/queue/retry/release/evidence/provider-contract CLI를 맡는다.
- Node.js는 ChatGPT 웹 UI/CDP/DOM/screenshot/composer/upload/turn/artifact download provider를 맡는다.
- Bash는 compatibility shim, legacy sensor, build/test helper로만 남긴다. production 판단을 Bash에 더 키우지 않는다.
- 큰 단일 파일을 계속 키우지 않는다. 책임 경계가 보이면 모듈로 나눈다.
- `src` 바로 아래 flat 파일을 계속 늘리지 않는다. `request`, `cli`, `runtime`, `provider`, `session`, `release`처럼 책임별 폴더 모듈을 우선 검토한다.
- top-level `src/*.rs`는 crate entrypoint, stable public facade, 작고 독립적인 core module에 한정한다.
- 폴더만 만든 뒤 큰 파일을 그대로 방치하지 않는다. SOLID/Clean Code와 Rust best practice에 맞게 실제 책임도 쪼갠다.
- request orchestration은 run loop, session persistence, terminal confirmation, artifact recovery, release cleanup, provider invocation, visual/wait gates가 분리되어야 한다.
- CLI는 dispatcher, argument parsing, output schema, command behavior가 한 파일에 과도하게 섞이지 않게 한다.
- runtime은 probe/readiness와 stop/control/release side effect를 분리한다.
- 의미 없는 wrapper 계층은 만들지 않는다. 분리는 변경 위험 감소, 테스트 가능성, 상태기계 명확성, 책임 경계, 읽기 쉬움이 있을 때 한다.
- Rust 파일은 가능한 한 250 pure LOC 이하를 유지한다. 초과가 보이면 먼저 책임을 나눈다.
- Rust는 Rust best practices와 명확한 enum/state-machine/error taxonomy를 우선한다.
- Node provider는 Playwright 공식 API와 CDP/DOM evidence를 우선하고, transient href/url 저장에 기대지 않는다.

## 테스트 구조

- 새로 추가하거나 수정하는 테스트 파일 구조는 `src/...` 배치를 `tests/...` 아래에서 미러링한다.
- 예: `src/provider_client.rs` 관련 통합 테스트는 `tests/provider_client.rs`를 Cargo 진입점으로 두고, 세부 케이스는 `tests/provider_client/*.rs`에 둔다.
- 예: `src/request_artifacts.rs` 관련 테스트는 `tests/request_artifacts.rs`와 `tests/request_artifacts/*.rs` 구조를 쓴다.
- 폴더 모듈 예: `src/request/release.rs`의 테스트는 `tests/request/release.rs` 또는 `tests/request/release/*.rs`에 둔다.
- `tests/misc.rs`, `tests/random_cases.rs` 같은 잡다한 파일명으로 섞지 않는다.
- 새 테스트를 `tests/request_run_failures.rs` 같은 flat catch-all 파일에 계속 쌓지 않는다. 기존 flat 파일은 migration 중 가능한 범위에서 mirror 구조로 옮긴다.
- 새 동작의 focused test와 최소 구현은 Codex가 승인 설계에 따라 작성하고, 관련 전체 검증을 직접 실행한다.
- 큰 구조 변경은 Pro design/review에서 behavior-preserving mechanical move와 green evidence 계획을 먼저 확정하고, Codex가 승인 설계에 따라 구현한 뒤 별도 Pro 구현 리뷰를 받아야 한다. Pro coding artifact는 사용자가 명시적으로 요구했거나 Pro가 deliverable로 주장한 경우에만 별도 artifact-handling 계약의 대상이다.
- live QA count와 Pro review validity는 source/test/runtime/contract 변경 시 영향 범위에 맞게 reset한다. 전체 live matrix는 unchanged final source에서 3회 연속 PASS하고, timing/race/restart/cooldown/fairness/unknown-session/release-recovery처럼 반복이 실제 결함 탐지에 필요한 케이스만 각각 10회 PASS한다. deterministic/static/schema 케이스에 임의의 10회 반복을 부과하지 않는다.

## Live / Pro 작업 규칙

- live send, direct Playwright wait, `gptpro`, `gpt-webai-lifecycle run/resume/poll/show`,
  artifact download, 긴 unified exec wait 전에는 반드시 read-only screenshot + DOM/CDP evidence를 먼저 저장하고 직접 확인한다.
- UI가 실제로 생성/생각/응답 진행 중인지 확인하기 전에는 기다리지 않는다.
- root idle composer, 과거 완료 화면, 빈 화면, 로그인/limit modal, 진행 증거 없는 상태이면 기다리지 말고 분류/회수/해제를 진행한다.
- 현재 Rust `preflight --json --docker-slot-provider`는 live send 전 필수 screenshot+DOM gate로 사용한다.
- `show --json --session`은 persisted-record-first recovery read path다.
- 같은 ChatGPT session 회수는 sessionId->slotId pinning을 지켜야 하며 fresh slot을 임의 배정하지 않는다.
- Pro 설계/리뷰/다운로드는 wrapper 출력만으로 성공 처리하지 않고, 직접 Playwright/CDP evidence와 artifact 파일 검증을 남긴다.
- 요청 모델이 `Pro`인데 최초 composer가 `Instant`, `Extra High` 또는 다른 모델인 것은 곧바로 `model.selection_mismatch`가 아니다. 전송 전 실제 Chrome screenshot+sanitized DOM/CDP로 현재 라벨을 관찰하고, picker를 열어 visible/available `Pro`를 선택한 뒤 새 screenshot+DOM/CDP에서 실제 composer 라벨 `Pro`를 재검증하는 것이 기본 동작이다. picker DOM drift/stale control은 bounded 재탐색·재캡처·필요한 scroll로 회복한다. mismatch/auth.needs_pro는 picker를 실제로 열어 확인했는데도 Pro가 없거나 bounded 선택 재검증이 실패한 경우에만 허용한다. 그 경우에도 다른 healthy cohort로 회전하고, 모두 불가하면 hard fail 대신 증거가 있는 recovering/queued로 남기며 silent downgrade하지 않는다.
- 현재 physical account cohort 설정은 `slot-01..03=cohort-a`, `slot-04..07=cohort-b`, `slot-08..10=cohort-c`다. legacy `group-01/group-02`를 deployment topology 외에 account fairness, provider-limit/cooldown, authorization 권위로 사용하지 않는다. 기존 profile을 재로그인·이동·복제·reseed하지 않는다.
- auth/root redirect/`session.content_unavailable`/provider-limit/bottom-proof도 관찰 -> bounded 회복 -> 새 screenshot+DOM/CDP 재검증 순서를 따른다. sessionId가 생긴 뒤에는 원 session/slot에 pin하고 매 poll/resume 전에 실제 pinned Chrome screenshot과 DOM/CDP를 확인한다.
- GPT Pro의 역할은 설계자와 독립 설계/구현 리뷰어다. Codex의 역할은 구현자/프로젝트 관리자/프로젝트 오너/시니어 QA다.
- Pro도 실수할 수 있으므로 `PATCH_READY`, LGTM, 본문 설명, 파일명, sha256 텍스트, inline diff를 그대로 믿지 않는다. screenshot, DOM/CDP, clicked download control, host-saved artifact, zip magic, sha256, manifest, resulting diff, tests, live QA evidence를 하나씩 대조한다.
- 사용자가 명시적으로 요구했거나 Pro가 deliverable로 주장한 artifact는 visible button/link를 클릭하고 Playwright download event로 host evidence/artifact dir에 저장한다.
- artifact 결과는 0/1/N개 배열로 저장하고 `{buttonText, buttonTextSha256, turnScope, clickedElement, artifact}` 형태의 객체 metadata를 남긴다.
- Pro 설계 산출물과 review verdict는 source/evidence bundle과 함께 보존한다. Pro에게 구현 artifact를 요구하는 대신 Codex가 승인 설계를 구현하고 검증한다.
- Codex 구현/검증이 실패하면 root cause를 분류해 승인 설계 범위에서 직접 수정한다. 설계·계약 변경이 필요하면 새 evidence를 묶어 Pro 설계 수정과 독립 재리뷰를 요청한다.
- Pro 작업은 3시간 이상 걸릴 수 있다. 실제 진행 중임을 screenshot+DOM/CDP로 확인한 같은 session은 cancel/interrupt/refresh/resubmit/session switch/doctor/임의 클릭/다른 UI retry로 방해하지 않는다.
- provider-limit modal의 `Got it`/닫기 버튼을 누른 것만으로 회복 판정하지 않는다. 닫은 뒤 새 screenshot+DOM/CDP gate와 짧은 canary send가 실제로 성공해야 재사용 가능으로 본다.
- provider-limit detection은 전체 body text를 훑지 않는다. 사용자 prompt, assistant answer, sidebar/history, attachment filename, composer text에 limit 문자열이 있어도 provider state가 아니다. visible blocking modal/toast/alert UI에서만 후보로 잡고, 이후 send-start evidence와 final artifact로 확정한다.
- provider-limit fallback은 preferred cohort 다음의 서로 다른 healthy cohort들을 결정적 순서로 시도한다. 모든 cohort가 `provider.limit`이면 승인 설계의 bounded cooldown 뒤 세-cohort 시퀀스를 새 screenshot+DOM/send evidence로 다시 검증하며, 실패를 성공으로 세지 않는다.

## Release / Recovery 규칙

- send 성공은 sessionId, non-root `/c/...` URL, target/session mapping, active turn/generation evidence, final answer artifact가 모두 있어야 한다.
- ChatGPT가 active turn/generation을 먼저 보여주고 `/c/...` URL이 늦게 따라올 수 있으므로 start-confirmation은 둘을 같은 bounded window에서 함께 기다린다. 단 timeout 시 root URL + active generation만으로는 여전히 `session.start_unconfirmed`이며 성공이 아니다.
- root URL, empty output, sessionId+long poll만 있는 상태, truncated raw, missing answer json, 반복 recovery_incomplete는 성공이 아니다.
- failed/unconfirmed/timeout/terminal success 모두 answer/artifact 저장 후 release가 holder/lock/runtime/container 상태를 정리해야 한다.
- clean release는 `holders=0`, `locks=0`, lifecycle-owned runtime stopped, 사용 slot `standby`/`exited`/`allocatable` 또는 explicit blocked state이며 stopped runtime을 `ready`로 보고하지 않는다.
- runtime stop은 명시적으로 검증된 경로에서만 수행한다. fake tests에서는 fake docker/provider를 사용하고, 실제 container stop은 의도와 evidence를 확인한 뒤에만 한다.
- stale locks는 recovery evidence를 남기고 정리한다. compaction 후 show/resume by session key가 동작해야 한다.

## 문서와 Evidence

- README, SMOKE_TESTS, runbook, AGENTS, local/global Codex docs는 auth gate, live QA, send-confirmation gate,
  release-after-use, session recovery, Pro design/review workflow, screenshot+DOM evidence rule, no-merge rule을 반영해야 한다.
- 각 중요한 fix/milestone은 evidence dir에 command, rc, stdout/stderr, summary, 관련 state/status, artifact sha256을 남긴다.
- 최종 완료는 unchanged-source 전체 live matrix 3회 연속 PASS, 반복 검증이 필요한 위험 케이스별 10회 PASS, 최종 implementation Pro `LGTM/no-blocking` 1회, docs 업데이트, clean release/status evidence, intended commit push가 모두 확인된 뒤에만 가능하다.
