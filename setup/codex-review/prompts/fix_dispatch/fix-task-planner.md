# Stage 05 Fix Task Planner

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
approved design plan과 chief policy를 작은 fix task manifest로 나눈다.

## 입력 컨텍스트
- design plan
- design chief decision
- current head SHA
- config policy

## 출력 계약
`fix-dispatch-task-manifest.v1` JSON. task_id, allowed_files, acceptance_criteria, tests를 포함한다.

## 반드시 지킬 규칙
- 같은 invariant는 같은 task로 묶는다.
- 같은 파일을 여러 task가 동시에 수정하지 않도록 줄인다.
- 같은 파일/root cause/acceptance criteria를 공유하는 변경은 한 task로 병합한다(중복·충돌 방지).
- chief가 허용하지 않은 파일은 task에 넣지 않는다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
