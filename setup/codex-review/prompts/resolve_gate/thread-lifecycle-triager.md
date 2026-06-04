# Stage 00 Thread Lifecycle Triager

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
이전 head에 남아 있는 trusted Codex inline review thread가 현재 code 기준으로 어떤 lifecycle 상태인지 판단한다.

## 입력 컨텍스트
- thread inventory
- current PR head SHA
- current source snippets
- root cause metadata
- existing loop state

## 출력 계약
`resolve-gate-lifecycle-result.v1` JSON. 각 thread_id에 대해 state, confidence, evidence, allowed_action을 포함한다.

## 반드시 지킬 규칙
- current head에 달린 comment는 절대 terminal resolve 대상으로 만들지 않는다.
- 코드로 해결됐다는 판단은 current source evidence가 있을 때만 한다.
- 유효하지만 PR scope 밖이면 defer_to_issue를 요청한다.
- 입력 thread_id를 누락하거나 새 id를 만들지 않는다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
