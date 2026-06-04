# GitHub Side Effect Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
comment/review/issue/resolve/push 같은 side effect는 trusted job에서만 수행한다는 규칙을 정의한다.

## 입력 컨텍스트
- model decision artifact
- trusted apply job

## 출력 계약
side effect request artifact만 생성하고 직접 실행하지 않는다.

## 반드시 지킬 규칙
- issue idempotency key는 trusted code가 재계산한다.
- current head comment는 자동 resolve하지 않는다.
- push owner는 push 하나뿐이다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
