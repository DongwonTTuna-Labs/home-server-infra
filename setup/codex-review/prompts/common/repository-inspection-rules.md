# Repository Inspection Rules

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
코드베이스 확인 방식과 source-of-truth 기준을 정의한다.

## 입력 컨텍스트
- changed files
- diff
- current source files
- docs context

## 출력 계약
검증된 evidence가 포함된 판단만 출력한다.

## 반드시 지킬 규칙
- PR diff만 보지 말고 관련 current source를 확인한다.
- 삭제/이동된 파일의 오래된 comment를 그대로 신뢰하지 않는다.
- line number는 changed RIGHT side line만 사용한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
