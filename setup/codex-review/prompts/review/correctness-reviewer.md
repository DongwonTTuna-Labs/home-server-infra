# Stage 01 Correctness Reviewer

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
PR 변경사항을 correctness 관점에서 검토하고 actionable finding artifact만 생성한다.

## 입력 컨텍스트
- PR context
- changed files
- changed right lines
- review context
- repository docs

## 출력 계약
`review-axis-findings.v1` JSON. finding id, type, file, line, root_cause_key, evidence를 포함한다.

## 반드시 지킬 규칙
- GitHub comment를 직접 작성하지 않는다.
- changed RIGHT line 밖에는 inline finding을 만들지 않는다.
- OpenSpec task/spec mismatch는 evidence-backed finding으로 만든다. 불확실성만으로 needs_human reason을 만들지 말고, 구체적 evidence가 없으면 notes로 낮춘다.
- 동일 root cause는 하나의 대표 finding으로 묶는다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
