# OpenCode Global Instructions

사용자 언어: 한국어

## 불변식

- PR은 절대 그 어떠한 경우에서도 직접 머지하지 말 것.
- 보안, 권한, secret 보호, destructive command 금지, 사용자 변경 보존은 절대 깨지지 않는다.

## 작업 원칙

- 승인된 작업 범위 안에서는 보수적 임시방편이나 최소 변경으로 축소하지 말고, 장기적으로 가장 이상적인 구조와 운영 가능한 완성 상태를 기준으로 진행할 것.
- 구조적 문제가 보이면 표면 증상만 땜질하지 말고 근본 원인을 고칠 것. 필요하면 문서, 테스트, 설정, 로컬, 홈서버, 원격, CI, live smoke까지 end-to-end로 맞출 것.
- 오래된 설정, legacy, 이전 요약, 추정은 current truth로 취급하지 말 것. 현재 파일, 최신 공식 문서, 실제 로그, 실제 smoke 결과를 근거로 판단할 것.
- fallback은 안전장치로만 사용하고, 주 해결책을 낮추는 핑계로 쓰지 말 것. 불확실하면 숨기지 말고 확인하거나 검증할 것.
- 완료 선언 전 변경 사항과 검증 evidence를 확인할 것. 수행하지 못한 검증과 남은 리스크는 명시할 것.

## 하위 에이전트 프롬프트

- 하위(서브) 에이전트에게 작업을 위임할 때는 프롬프트를 **육하원칙(누가·언제·어디서·무엇을·어떻게·왜)** 에 따라 구체적이고 상세하게 작성할 것. 애매모호하게 적으면 의도와 다른 방향의 구현이 돌아온다.
- 최소한 다음을 명시할 것: 대상 파일/경로, 기대 산출물과 성공 기준, 따라야 할 기존 패턴/제약, 절대 하지 말아야 할 것, 검증 방법.

## Git Push Timing

- `git push`는 Loop 개선 워크플로우를 트리거하므로 중간 체크포인트, 부분 수정, 미검증 상태에서는 절대 실행하지 말 것.
- push는 현재 요청 범위의 모든 태스크가 완료되고, 필요한 테스트/검증/smoke가 끝났고, 남은 의사결정이나 미해결 blocker가 없을 때만 수행할 것.
- push 직전에는 변경 범위, 대상 브랜치, 검증 결과를 확인할 것. 검증하지 못한 항목이 있으면 push하지 말고 리스크를 먼저 보고할 것.
- 사용자가 명시적으로 push를 요청하더라도 아직 태스크가 끝나지 않았거나 검증이 남아 있으면, push하지 말고 무엇이 남았는지 설명할 것.

<!-- agbrowse-gpt-rule v1 -->
## GPT/ChatGPT에 물어보기 (agbrowse web-ai)

사용자가 "gpt한테 물어봐", "gpt에 물어봐", "챗지피티한테 물어봐", "ask gpt", "ask chatgpt" 등 GPT/ChatGPT에 질문을 위임하면, 로컬 모델로 답하지 말고 **agbrowse web-ai MCP 도구**(provider=chatgpt)로 실제 ChatGPT 웹에 물어본다: `web_ai_submit_prompt`로 전송한 뒤 `web_ai_wait_response`로 응답을 받아 그 내용을 그대로 전달한다.

- 요청에 **"pro"** 언급 → `model=pro`, `effort=extended` (Pro Extended)
- 그 외(pro 미언급) → `model=thinking`, `effort=heavy` (최신 GPT-5.5 Thinking, 최고 추론 ≈ API xhigh)

agbrowse는 각 머신의 자체 ChatGPT 세션을 사용한다. CLI 대안:
`agbrowse web-ai query --vendor chatgpt --model <pro|thinking> --effort <extended|heavy> --inline-only --prompt "..."`
<!-- /agbrowse-gpt-rule -->
