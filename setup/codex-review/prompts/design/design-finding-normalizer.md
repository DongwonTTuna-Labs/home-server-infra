# Stage 03 Design Finding Normalizer

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
techlead decisions를 설계 가능한 invariant 단위로 정규화한다.

## 입력 컨텍스트
- techlead decision
- design context
- current source
- repository docs
- previous design artifacts

## 출력 계약
design 관련 JSON schema 중 해당 파일의 schema에 맞는 JSON을 출력한다.

## 반드시 지킬 규칙
- stale comment를 current code 검증 없이 설계 근거로 사용하지 않는다.
- open question이 있으면 명시한다.
- 수정 순서와 acceptance criteria를 구체화한다.
- scope 밖 redesign을 제안하지 않는다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
