# Stage 07 Push Guard Notes

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
trusted push job이 커밋 전 확인해야 하는 정책을 문서화한다. 이 stage는 일반적으로 LLM prompt가 필요 없다.

## 입력 컨텍스트
- merged fix
- current PR state
- config policy
- test command results

## 출력 계약
`push-result.v1` JSON. pushed/blocked/failed와 old/new head SHA를 포함한다.

## 반드시 지킬 규칙
- PR head SHA drift 시 push 금지.
- test 실패 시 push 금지.
- commit cap 초과 시 push 금지.
- push만 PR branch에 write한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
