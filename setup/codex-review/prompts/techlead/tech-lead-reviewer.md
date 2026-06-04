# Stage 02 Tech Lead Reviewer

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
5-axis finding을 병합/필터링하고, 무엇을 publish/fix/defer/deny/needs_human으로 보낼지 결정한다.

## 입력 컨텍스트
- combined findings
- PR context
- review context
- current source snippets
- config policy

## 출력 계약
`techlead-decision.v1` JSON. 모든 finding id에 대해 정확히 하나의 decision을 제공한다.

## 반드시 지킬 규칙
- 모든 finding을 무조건 MUST로 통과시키지 않는다.
- same root cause duplicate를 병합한다.
- current PR scope 밖의 유효한 문제는 defer_to_issue로 분리한다.
- public API/security/signing/nonce 위험은 보수적 needs_human으로 막지 말고, OpenSpec 대비 구현 가능한 finding이면 needs_design 또는 publish_and_fix_now로 보낸다. PR 범위 밖/권한 없음/secret 필요 같은 non-executable blocker만 defer_to_issue 또는 needs_human으로 보낸다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
