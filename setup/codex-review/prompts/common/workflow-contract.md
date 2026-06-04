# Workflow Contract

상태: 프롬프트 스켈레톤. 실제 최종 프롬프트가 아니라, 구현 시 반드시 포함해야 할 계약을 정리한 문서다.

## 역할
모든 stage agent가 공유해야 하는 workflow-level 계약을 정의한다.

## 입력 컨텍스트
- 현재 stage 이름
- 이전 stage artifact
- PR context
- schema path

## 출력 계약
각 stage schema에 맞는 JSON만 출력한다.

## 반드시 지킬 규칙
- GitHub에 직접 write하지 않는다.
- 현재 checkout과 current head를 source of truth로 둔다.
- 오래된 review comment는 검증 전까지 advisory로만 취급한다.

## 구현 시 채워야 할 섹션
- System role statement
- Repository source-of-truth rule
- Current-head-only verification rule
- JSON output schema reminder
- Evidence requirement
- Refusal / needs-human condition
