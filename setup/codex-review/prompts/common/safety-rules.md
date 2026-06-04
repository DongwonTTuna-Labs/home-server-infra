# Safety Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
보안/권한/secret/side effect 금지 규칙을 정의한다.

## 입력 컨텍스트
- artifact bundle
- token availability
- repo policy

## 출력 계약
기계적으로 실행 불가능한 경우만 needs_human/no_safe_fix로 분리하고, 의미론적 위험은 evidence와 테스트 요구사항으로 남긴다.

## 반드시 지킬 규칙
- secret-like material 출력 금지.
- model job에서 GitHub write 금지.
- public API/signing/auth/nonce/live-capable 같은 의미론적 위험은 자동 fix 금지가 아니라 high-risk evidence, acceptance criteria, required tests로 다룬다. secret 생성, write side effect, 권한/정책 위반만 하드 차단한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
