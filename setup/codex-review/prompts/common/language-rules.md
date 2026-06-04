# Language Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
모델 응답의 언어와 표현 방식을 통일한다.

## 입력 컨텍스트
- 출력 대상: comment/body/summary/json reason

## 출력 계약
사람이 읽는 문장은 한국어, machine field는 stable enum/string을 사용한다.

## 반드시 지킬 규칙
- JSON key는 영어 snake_case를 유지한다.
- 사용자-facing summary는 한국어로 작성한다.
- OpenSpec-backed 작업에서 불확실성은 evidence/acceptance criteria로 좁히고, generic needs_human으로 도망가지 않는다. needs_human은 명시적 non-executable blocker가 있을 때만 사용한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
