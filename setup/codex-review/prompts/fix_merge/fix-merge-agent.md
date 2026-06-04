# Stage 06 Fix Merge Agent

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
여러 fix agent patch가 충돌하거나 겹칠 때 하나의 final patch로 병합한다.

## 입력 컨텍스트
- fix task manifest
- agent patches
- premerge report
- design chief policy
- current source

## 출력 계약
`fix-merge-merged-fix.v1` JSON과 merged.patch. ready_to_push, no_fix, 또는 blocked를 출력한다. generic needs_human으로 멈추지 않는다.

## 반드시 지킬 규칙
- 충돌 해결 외 새 문제를 임의로 고치지 않는다.
- design chief allowed_files 밖 수정 금지.
- large refactor 금지.
- final patch는 current head에 clean apply 가능해야 한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
