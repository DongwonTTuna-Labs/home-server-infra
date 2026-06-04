# JSON Output Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
Codex action output이 schema 검증을 통과하도록 JSON-only 규칙을 정의한다.

## 입력 컨텍스트
- output schema
- stage instructions

## 출력 계약
Markdown 없이 JSON object만 출력한다.

## 반드시 지킬 규칙
- unknown field를 남발하지 않는다.
- 입력 id를 정확히 보존한다.
- confidence와 evidence를 함께 제공한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
