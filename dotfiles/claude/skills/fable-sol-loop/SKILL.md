---
name: fable-sol-loop
description: Fable이 기획서를 작성·수렴시킨 뒤 codex(gpt-5.6-sol ultra)를 부려 구현하고 검증 루프를 도는 멀티모델 워크플로. 사용자가 "샤바샤바", "fable-sol-loop", "솔한테 시켜", "codex로 구현해"라고 하거나 기획→구현→검증을 통째로 맡길 때 사용.
---

# Fable–Sol Loop

Fable(이 세션)이 기획과 검증을 맡고, codex의 `gpt-5.6-sol`(ultra)이 구현을 맡는 워크플로.
핵심 원칙: **인풋이 쓰레기면 구현도 쓰레기다.** Fable의 토큰은 코드 작성이 아니라
기획서·지시서·검증의 품질에 쓴다. 출력은 기본 한국어.

## 역할 분담

- **Fable (이 세션)** — 기획자, 오케스트레이터, 최종 판단자. 구현 코드를 직접 짜지 않는다.
  요구 정리 → 기획서 작성·수렴 → codex 지시 → 결과 검증 → 완료 판정.
- **codex `gpt-5.6-sol` (ultra)** — 구현 담당. 답이 정해진 구현·백엔드·반복 작업에 강함.
  기획서가 상세할수록 결과 품질이 급이 달라진다.
- **ChatGPT Pro (홈서버 `gpt-webai-pro` 슬롯 데몬)** — 외부 리뷰어. Phase 1.5에서 기획서를,
  Phase 3에서 구현 결과를 LGTM까지 리뷰한다 (둘 다 기본 ON). **경로 갱신(2026-09-01): 유일한
  경로는 홈서버 gpt-webai-pro CLI다** (`ssh home`, 전역 `~/.claude/rules/gpt-delegation.md`
  1순위와 동일). chatgpt-pro-ask(맥 화면 자동화)는 이 루프에서 사용하지 않는다.
- **사람** — 방향 결정자. 기획 확정 직전에 반드시 1회 체크포인트를 갖는다
  (단, 사용자가 autonomous로 전체 위임한 경우 생략 가능).

## 환경 사실 (2026-08-23 갱신)

- **홈서버(linux)**: `codex`는 `/home/dongwonttuna/.local/bin/codex`, provider는 로컬 codex-lb(`127.0.0.1:2455`).
  effort는 `medium/high/xhigh/max/ultra` 지원 — 구현 turn은 `ultra`.
- **맥북(macOS)**: `codex`는 `/Users/dongwon/.local/bin/codex`, provider는 릴레이 codex-lb
  (`relay-ai.dongwontuna.net`, `~/.claude/rules/codex-lb-models.md` 참조). **effort는 사용자 지시로 `max`를 쓴다.**
- 전역 `~/.codex/config.toml`이 이미 `approval_policy="never"` + `sandbox_mode="danger-full-access"`이므로
  `codex exec`는 추가 위험 플래그 없이 자동 실행된다. `--dangerously-*` 플래그를 덧붙이지 말 것.
- drift에 대비해 호출 시 항상 `-m <모델> -c model_reasoning_effort=...`를 명시한다.
- **모델 전환 (2026-09-05 사용자 지시)**: 구현·리뷰 기본 모델은 **`gpt-6-astra` / effort `max`** 다(맥·홈서버 `~/.codex/config.toml` 기본값도 전환됨).
  이 문서의 `-m gpt-5.6-sol … ultra|max` 예시는 전부 `-m gpt-6-astra -c model_reasoning_effort='"max"'`(러너는 `--model gpt-6-astra --effort max`)로 읽는다.
  sol은 사용자가 명시할 때만 쓴다. 능력 비교 실측은 아직 없다(`~/.claude/rules/codex-lb-models.md`).
- **네트워크 전환(와이파이 변경 등) 중 codex exec가 조용히 멈출 수 있다** — 프로세스는 살아 있는데
  로그 파일이 더 이상 자라지 않는 형태(2026-08-23 실측: 20분 정지). 장시간 turn은 로그 mtime을
  감시하고, **5분 이상 0바이트 증가면 stall로 판정**해 죽이고 재실행한다. 읽기 전용 리뷰 turn은
  같은 프롬프트 재실행이 안전하고, 구현 turn은 새 세션이 아니라 같은 스레드를 resume한다.
- **codex를 감시 루프와 같은 셸/프로세스 그룹에서 띄우지 마라** (2026-08-23 실측 사고 2회):
  배경 태스크가 어떤 이유로든 죽으면(하니스 태스크 정리, Esc/Ctrl+C) 자식 codex가 동반 종료된다.
  반드시 `nohup codex exec ... & disown`(또는 `setsid`)으로 **detach해서 띄우고**, 감시는 별도
  태스크에서 보고서 파일 존재·로그 mtime만 폴링한다. 감시자가 죽어도 codex는 계속 돈다.
  Claude Code에서는 detach 실행과 감시 루프를 **서로 다른 Bash 백그라운드 호출**로 분리할 것.

## Phase 0 — 과제 정의

사용자 요구를 다음 4개로 정리해 짧게 보여준다: **목표 / 성공 기준(검증 가능한 형태) / 제약 / 비범위(non-goals)**.
모호하면 여기서 사용자에게 물어본다. 이 단계를 건너뛰고 기획서를 쓰지 않는다.

## Phase 0.5 — 작업 단위 쪼개기 (필수)

**원칙: 1 단위 = 1 sol turn = 1 PR = 감독자가 한 자리에서 리뷰 가능한 크기.**

### 상한 (초과 시 반드시 분할)

마이그레이션 1개 · SQL ≤ 1,500줄 · 변경 파일 ≤ 30 · 신규 crate/service ≤ 1 ·
신규 화면 ≤ 1 · **새 권위/계약 패키지 ≤ 1** · 기획서 대과업(T 항목) ≤ 2.

초과 요구는 기획 단계에서 **의존 순서가 있는 N개 단위로 분해**한다.
"한 번에 하면 빠르다"는 착시다 — 리뷰 실패로 되돌아온다.

### 어떻게 자르는가 — 절단 기준

1. **가로(기능)로 자르고 세로(레이어)로 자르지 않는다.** 한 조각이 스키마→소유 함수→
   서비스→테스트까지 관통해야 그 조각만으로 게이트를 돌릴 수 있다. "DB만 먼저, Rust는
   다음에"는 검증 불가능한 조각을 만든다.
2. **호출 그래프에서 끊는다.** 서로 호출하지 않는 블록은 이미 별개 단위다.
3. **fail-closed latch가 천연 절단선.** 첫 줄에서 에러를 던지는 블록은 인바운드·아웃바운드가
   없으므로 통째로 격리해도 나머지에 영향이 0이다.
4. **승인 대기(권위·계약 결정)는 1개씩.** 여러 개를 한 단위에 넣으면 서로를 막고 승인
   문서가 수천 줄로 부푼다.
5. **집합 검증(postcondition assert)은 마지막 조각 전용.** 권한 집합 동등성 같은 검사는
   중간 조각에 복제하면 전부 실패한다.
6. **카운트·인벤토리 계약은 분할과 같은 조각에서 갱신.** 테이블/함수/구문 수를 고정한 계약이
   있으면 그 갱신을 미루는 순간 게이트가 깨진다.
7. **최소 단위는 "검증 가능성"이 정한다.** 그 조각만으로 게이트를 못 돌리면 너무 잘게 자른
   것이다. 더 못 자르면 그게 바닥이다.

### 길어지는 것을 조기에 잡는 신호 (하나라도 걸리면 즉시 분할·중단)

- 한 단위에서 sol의 `OPEN_QUESTIONS`가 **2회 이상** 나온다 → 단위가 너무 크다는 확정 신호.
- 같은 단위가 **sol turn 3회**를 넘어간다.
- 단일 산출물이 상한의 2배를 넘는다(예: 마이그레이션 3,000줄 초과).
- 감독자 리뷰에서 **매번 새 결함**이 나온다 → 이미 리뷰 가능 크기를 넘었다.

실행 중 초과를 감지하면 **즉시 steer**해 현재 범위에서 일관된 상태로 마무리시키고
(반쯤 열린 계약을 남기지 않는 선에서) 나머지는 다음 단위로 이월한다.

**실증 근거**: 구린내 R6e가 한 단위에 대과업 4개를 담아 19,245줄 마이그레이션 +
권위 패키지 4종 동시 승인 대기(제안서만 11,313줄) 상태가 됐고, 매 리뷰마다 신규 결함이
나왔다. 원인은 구현 품질이 아니라 단위 크기였다.

## Phase 1 — 기획 루프 (가장 중요)

1. 작업 리포지토리 루트에 `.fable-sol/plan.md` 기획서를 작성한다. 필수 섹션:
   - 배경과 목표
   - 아키텍처 결정과 근거
   - **파일 단위 변경 목록** (경로, 무엇을 어떻게)
   - 테스트 계획 (어떤 테스트를 새로 쓰는지)
   - **완료 기준** — codex가 스스로 실행할 수 있는 정확한 검증 명령 포함
   - 금지 사항 (스코프 밖 파일, 건드리면 안 되는 설정 등)
   - **이 단위의 크기 추정** — 변경 파일 수 · SQL 줄수 · 신규 개념(권위/계약/crate/화면) 수.
     Phase 0.5 상한을 넘으면 기획 단계에서 분할한다 (기획 시점에 자각하는 것이 목적).
2. 작성 후 Fable 스스로 최소 1회 비판적 재검토를 돌린다:
   빠진 엣지케이스, 두 갈래로 해석되는 지시, 검증 불가능한 완료 기준, 암묵적 가정을 찾아 기획서를 고친다.
   수렴할 때까지 반복한다.
3. **사용자 체크포인트**: 기획서 요약(변경 파일 목록 + 완료 기준)을 보여주고 방향 승인을 받은 뒤
   Phase 1.5로 간다 (autonomous 전권 위임 시 생략 가능). Pro 리뷰는 라운드당 수 시간이 들므로,
   방향 승인을 먼저 받고 나서 리뷰를 돈다.

## Phase 1.5 — ChatGPT Pro 설계 리뷰 루프 (기본 ON)

수렴된 기획서를 **홈서버 `gpt-webai-pro` 슬롯 데몬**(Pro 추론 수준)으로 보내 외부
설계 리뷰를 받고, 실질 blocker만 반영하며 `<verdict>LGTM</verdict>`이 나올 때까지 루프한다. **기본 ON.**
스킵은 (a) 사용자가 명시적으로 지시했거나("pro 리뷰 스킵"), (b) 변경이 명백히 소규모·기계적이라
Fable이 체크포인트에서 스킵을 제안해 승인받은 경우만. 전역 rule `~/.claude/rules/gpt-delegation.md`의
불변식을 그대로 따른다: 오래 걸리는 것은 실패가 아니고, timeout·send_uncertain은 **재전송이
아니라 같은 sessionId의 `resume`으로만** 잇는다. fresh 적대 리뷰이므로 `--conversation` 없이
새 채팅으로 보낸다. RouteFork 작업은 `GWP_ONLY_SLOT=slot-a` 고정(프로젝트 rule). 표준 호출:

```bash
ssh home 'mkdir -p ~/rf_work/<unit>'
scp context.zip prompt.md home:rf_work/<unit>/
ssh home 'export PATH="$HOME/.local/bin:$PATH" GWP_ONLY_SLOT=slot-a; cd ~/rf_work/<unit>; \
  setsid nohup bash -c "cat prompt.md | gpt-webai-pro run --file ~/rf_work/<unit>/context.zip \
  > envelope.json 2>run.log" </dev/null >/dev/null 2>&1 &'
# 회수: envelope.json(status·answerPath). 감시는 로컬에서 3분 간격 짧은 ssh 폴링
# (홈서버가 장수 SSH를 끊으므로 tail -f 금지). 2026-09-01 ee855a3a 이후 run/resume는
# 소유자 생존 시 attach-대기하므로 envelope은 종료 상태에서만 쓰인다 — 폴링은 파일 존재가
# 아니라 터미널 상태(complete/needs_user_action/failed) 기준, `running`이 보이면 요청
# 디렉토리(~/.local/state/gpt-webai-pro/requests/<id>/answer.md)로 재판정.
```
리뷰 라운드가 도는 동안 Phase 2에 선진입하지 않는다 (리뷰가 뒤집을 수 있는 구현에 시간을 태우지 않는다).

### 라운드 프로토콜

각 Pro 요청은 fresh 리뷰어다 (멀티턴 대화가 아님). 컨텍스트 연속성은 세션이 아니라
프롬프트에 담아 보장한다:

1. **로컬 적대 사전 리뷰** (1라운드 전송 전 필수): fresh-context 서브에이전트에게 아래
   materiality bar와 동일한 기준으로 적대 리뷰를 시켜 접합부 결함을 먼저 잡는다.
   외부 왕복 1회(수 시간)보다 훨씬 싸다.
2. **전송 내용**: 프롬프트 본문에는 (a) materiality bar의 BLOCKING 기준,
   (b) 판정 형식 지시 — 응답 마지막에 `<verdict>LGTM|BLOCKERS</verdict>`, blocker는
   `<blocking_issues>`에 항목별 (문제 / 근거 / BLOCKING인 이유), 그 외 제안은
   `<non_blocking_notes>`로 분리, (c) 2라운드부터는 이전 라운드 blocker별 처리 결과
   (반영 내역 또는 기각 사유)를 담고 "기각 항목을 같은 논거로 재제기하지 말고,
   처리 결과 검증과 신규 blocker만 보고하라"고 지시한다.
   **자료는 전체 컨텍스트 zip 하나로 `gpt-webai-pro run --file`에 첨부한다**: 기획서 전문(현재 버전)만이 아니라, 리뷰어가
   배경→과정→결과를 재구성할 수 있도록 존재하는 모든 자료 — 관련 소스 스냅샷, 설계서·설계
   리뷰 원문, `.fable-sol/review-log.md`와 이전 라운드 리뷰 원문, scratchpad의 관련 검토
   기록 — 를 넣고, zip 루트 `MANIFEST.md`에 읽는 순서와 이번 판정 대상을 명시한다.
   secret(.env/키/토큰)과 `.git/`·`node_modules/`·빌드 산출물은 제외한다.
3. **분류·반영**: Fable이 blocker를 실질 / 검증 오바 / gold-plating으로 분류해 실질만
   기획서에 반영한다 (전면 전파 원칙 준수). 기각은 사유와 함께 기록한다 — GPT 피드백은
   과잉 보수적인 경향이 있으므로 이 필터가 필수다.
4. **리뷰 로그**: `.fable-sol/review-log.md`에 라운드마다 세션 id(`req_...`), verdict,
   blocker 목록과 분류·처리 결과를 기록한다. 다음 라운드 전송분 (d)의 원천이자,
   Phase 3 최종 게이트와 사후 감사용 evidence다.
5. LGTM까지 2→4를 반복한다.

### materiality bar (필수 — PR72에서 학습)

리뷰 프롬프트에 **blocking 기준을 명시적으로 캘리브레이션**한다. 기준을 "어디에도 두 갈래
해석이 없을 것"으로 두면 대형 스펙에서 fresh 리뷰어가 매 라운드 한 층 더 깊은 세부(해시 유도
바이트 순서, oracle 정의, 동시성 선형화 등)를 무한히 파내므로 루프가 수렴하지 않는다
(PR72에서 27→12→15→15→20으로 5라운드 비수렴 실증). 프롬프트에 넣을 기준:

> BLOCKING = (a) 명시된 acceptance/검증 명령이 작성된 대로 실행 불가·통과 불가이거나,
> (b) 구현이 실제로 두 갈래로 갈라져 상호 운용이 깨지는 결함만.
> 합리적인 단일 관례 선택으로 해소되는 해석 여지, 스타일, 추가 강화 아이디어는
> NON-BLOCKING으로 분류할 것.

- **전면 전파 원칙**: 리뷰 반영으로 전역 규칙을 추가하면 그 규칙이 영향을 주는 모든 closed
  스키마/표를 같은 라운드에 명시적으로 갱신한다. "규칙 문장만 추가"는 다음 라운드 blocker를 양산한다.

### 수렴·에스컬레이션·동결

- **비수렴 에스컬레이션**: blocker 수가 2라운드 연속 감소하지 않거나 3라운드 안에 LGTM이
  안 나오면 루프를 멈추고 사람에게 (1) 기준 재조정 (2) 설계 충분 선언 후 구현 진입
  (잔여 모호성은 구현자가 OPEN_QUESTIONS로 올리고 Fable이 즉답) 중 택일을 요청한다.
- **방향 변경 시 재체크포인트**: 리뷰 반영이 제품 방향 자체를 바꾸는 수준이면 자율 반영하지
  말고 사용자 체크포인트로 되돌린다 (Phase 2.5 에스컬레이션 기준과 동일).
- **LGTM 후 기획서 동결**: 이후 필요한 변경은 본문을 고치지 않고 addendum 절로만 추가한다
  (PR72 R23 방식). 리뷰받은 본문이 조용히 바뀌면 LGTM 판정 자체가 무효가 되기 때문이다.

## Phase 2 — codex 구현

1. 기획서 앞에 다음 "구현 계약"을 붙여 지시서를 완성한다:
   - 기획서의 파일 목록 밖 파일 수정 금지
   - **한국어 산출물 언어 규칙 (sol 고질병)**: 한국인이 실제 업무에서 쓰는 자연스러운 한국어로만
     작성한다. 현지에서 정착된 외래어(버튼, 서버, 리뷰 등)는 허용하되, **현지인이 잘 쓰지 않는
     영단어·영어식 표현은 전부 한국어로 바꾼다.** sol은 이런 비정착 영단어를 남발해 문서 의미
     파악을 어렵게 만드는 경향이 있으므로, 지시서에 이 규칙을 명시하고 Phase 3 검수에서도
     같은 기준으로 지적·수정시킨다.
   - 테스트 없이 완료 선언 금지 — 완료 기준의 검증 명령을 직접 실행하고 결과를 보고할 것
   - 최종 메시지에 반드시 포함: 변경 파일 목록, 실행한 검증 명령과 결과, 남은 리스크
   - git commit/push 금지 (워킹 트리 변경까지만)
2. 실행 — 기본 경로는 **app-server 데몬 경유** (스킬 동봉 러너, Codex 앱에서 실시간 확인 가능):
   ```bash
   setsid nohup python3 ~/.claude/skills/fable-sol-loop/appserver-sol.py start \
     --cwd "<repo절대경로>" --model gpt-5.6-sol --effort ultra \
     --prompt-file "<repo>/.fable-sol/plan.md" \
     --out "<repo>/.fable-sol/last-message.md" --rc "<repo>/.fable-sol/run.rc" \
     > "<repo>/.fable-sol/run.log" 2>&1 & disown
   ```
   - 러너는 turn 시작 직후 `THREAD_ID=...`를 로그에 출력한다. **이 id를 `.fable-sol/state.md`에
     기록**한다 (fix 라운드 resume에 필수). thread id == codex session id이며 rollout은
     `~/.codex/sessions/`에 동일하게 저장된다.
   - turn은 app-server 데몬에 상주하므로 러너/터미널/Claude 세션이 죽어도 **계속 실행된다**
     (detach-safe 검증됨). 회수/확인: `appserver-sol.py wait|read|status --thread <ID>`.
   - 앱 가시성: 대화는 Codex 앱(데스크톱/모바일)에서 `--cwd`와 정확히 일치하는 워크스페이스
     폴더 아래에 실시간으로 보인다. `codex exec`는 source=exec라 앱 기본 목록에서 **숨겨지므로**,
     사용자가 앱에서 보고 싶어 하면 반드시 데몬 경로를 쓴다.
   - 데몬 확인: 소켓은 `~/.codex/app-server-control/app-server-control.sock`(WebSocket JSON-RPC v2).
     죽어 있으면 `codex app-server daemon start`로 살린다.
3. fallback (데몬 복구 불가 시에만) — 기존 headless 경로:
   ```bash
   codex exec -C "<repo절대경로>" -m gpt-5.6-sol -c model_reasoning_effort="ultra" \
     -o "<repo>/.fable-sol/last-message.md" - < "<repo>/.fable-sol/plan.md"
   ```
   이 경우도 session id를 회수해 `.fable-sol/state.md`에 기록한다.
   주의: exec 세션은 데몬에서 `resume`해도 source=exec가 유지되어 **앱에 안 보인다** (실측).
   앱으로 옮기려면 `thread/fork`로 인터랙티브 스레드를 만들어 이어간다 — 이후 모든 라운드는
   fork id로만 진행하고 원 세션은 retire한다 (히스토리 분기 방지).

## Phase 2.5 — 실행 중 감독 (mid-run supervision, 사용자 전권 위임)

sol turn이 도는 동안 Fable은 손 놓고 완료 통지만 기다리지 않는다. 사용자가 방향 판단
전권을 Fable에 위임했으므로, 다음을 자율적으로 수행한다:

- **주기 점검**: 긴 turn(ultra) 중 20-40분 간격으로 워킹트리 스냅샷(`git status`/`git diff --stat`),
  러너 로그, (가능하면) rollout 진행을 훑어 방향을 확인한다. background `sleep` 태스크를
  wakeup 타이머로 쓰면 세션이 알아서 재기동된다.
- **조기 개입 기준**: (a) 기획서/canonical 스코프 밖 파일 수정, (b) 같은 오류 반복·루프 정체,
  (c) 테스트 약화/삭제/skip, (d) 명백히 잘못된 방향의 대규모 diff, (e) 금지된 부수효과
  (커밋/push, live 접촉, 상태 파괴). 발견 즉시 개입한다 — turn 중단이 필요하면 중단하고
  fix 지시로 재개하는 것이 잘못된 방향으로 수 시간 태우는 것보다 싸다.
- **개입 사다리 (sol은 터널비전 성향 — 자기 방향으로만 쭉 가므로, 관찰로 끝내지 말고 반드시
  말을 걸어 교정한다)**:
  1. **steer** — turn을 살린 채 실행 중인 sol에게 교정 메시지를 주입한다 (경미한 방향 이탈,
     우선순위 재지정, "그 파일 건드리지 마" 류):
     ```bash
     printf '%s' "<교정 메시지>" | python3 ~/.claude/skills/fable-sol-loop/appserver-sol.py steer \
       --thread "<THREAD_ID>"
     ```
  2. **interrupt + 정리 + 재지시** — 방향이 명백히 틀렸거나 steer로 안 잡히면 turn을 중단하고,
     Fable이 워킹트리/상황을 직접 정리·판단한 뒤, 무엇이 왜 잘못됐고 어떻게 해야 하는지를
     담은 fix 지시서로 같은 스레드를 resume한다:
     ```bash
     python3 ~/.claude/skills/fable-sol-loop/appserver-sol.py interrupt --thread "<THREAD_ID>"
     # (정리·판단 후)
     printf '%s' "<fix 지시서>" | python3 ~/.claude/skills/fable-sol-loop/appserver-sol.py resume \
       --thread "<THREAD_ID>" --model gpt-5.6-sol --effort ultra --out ... --rc ...
     ```
  3. 같은 문제로 interrupt가 3회 반복되면 사람에게 에스컬레이션한다.
  - 스키마 참고: `turn/steer`는 `{threadId, expectedTurnId, input}`, `turn/interrupt`는
    `{threadId, turnId}` — 러너가 active turn id를 자동 조회해 채우며, active turn이 없으면
    안전하게 거부한다. interrupt된 turn은 `turn_aborted`로 기록되고 스레드/rollout은 보존된다.
- **본질 집중 / 과잉 쳐내기**: sol과 외부 리뷰어는 둘 다 과잉공학(gold-plating) 성향이 있다.
  "지금 우리가 본질적으로 뭘 만들고 있는가"를 기준으로, 목표 달성에 기여하지 않는 세부
  (불필요한 일반화, 쓰이지 않는 추상화, 검증 불가능한 요구, 스펙을 위한 스펙)는 Fable이
  전권으로 쳐내고 그 결정을 기록한다. 반대로 안전 불변식(보안, 사용자 변경 보존, fail-closed,
  merge 금지)은 절대 완화하지 않는다.
- **결정 즉답**: sol이 OPEN_QUESTIONS로 멈추면 사람에게 미루지 말고 Fable이 canonical/코드
  근거를 직접 읽고 decision-complete로 즉답해 같은 스레드를 재개한다 (PR72 R23 addendum 방식:
  승인 문서는 바이트 동결, 결정은 addendum으로). 사람 에스컬레이션은 (1) 안전 불변식 충돌,
  (2) 제품 방향 자체의 변경, (3) 비용/외부 리소스의 큰 확대일 때만.

## Phase 3 — 검증 루프 (Fable이 검증자)

1. `git status` / `git diff`로 변경을 Fable이 직접 리뷰한다:
   기획서 대비 누락, 스코프 이탈, 테스트가 실체인지(형식적 assert가 아닌지), last-message의 주장과 diff의 일치 여부.
2. 프로젝트 게이트(테스트/린트/빌드)를 Fable이 **직접 재실행**해 evidence를 확보한다.
   codex의 자기 보고만 믿고 통과 처리하지 않는다.
3. 문제 발견 시 fix 지시서(문제 → 기대 동작 → 재검증 명령)를 작성해 **같은 스레드에** 재지시한다:
   ```bash
   printf '%s' "<fix 지시>" | python3 ~/.claude/skills/fable-sol-loop/appserver-sol.py resume \
     --thread "<THREAD_ID>" --model gpt-5.6-sol --effort ultra \
     --out "<repo>/.fable-sol/last-message.md" --rc "<repo>/.fable-sol/run.rc"
   ```
   (fallback: `codex exec resume "<SESSION_ID>" -m gpt-5.6-sol -c model_reasoning_effort="ultra" ...`
   — thread id와 session id는 같은 값이다.)
   새 세션을 만들지 않는다 (컨텍스트 유실 방지). 통과할 때까지 루프하되,
   **동일 문제가 3회 반복되면 루프를 멈추고 사람에게 에스컬레이션**한다.
4. **최종 ChatGPT Pro 구현 게이트 (기본 ON — 스킵 규칙은 Phase 1.5와 동일)**: 게이트 LGTM 전에는
   완료를 선언하지 않는다. Phase 1.5와 같은 gpt-webai-pro 경로로 **전체 컨텍스트를 zip으로 첨부**한다
   base/head 커밋 정보, 기획서(+addendum), 설계서·설계 리뷰, `.fable-sol/review-log.md`와
   이전 리뷰 원문, 검증 evidence(실행 명령+출력 원문), zip 루트 `MANIFEST.md`.
   diff 요약만 보내는 축소 금지 — 리뷰어가 스냅샷 위에서 diff를 직접 읽고 배경·과정·결과를
   재구성할 수 있어야 한다. Phase 1.5와 동일한 라운드 프로토콜·envelope·materiality bar로
   판정받는다. blocker는 Fable이 과잉-보수 필터링 후 실질만 fix 루프(3번)로 되돌리고,
   라운드는 `.fable-sol/review-log.md`에 기록한다. 수렴·에스컬레이션 규칙도 Phase 1.5와
   동일하게 적용한다.

### 능동 수색 목록 — codex가 반복 생산하는 "껍데기 강제" 4종

diff 리뷰에서 **매번 이 4개를 명시적으로 찾아라.** 전부 게이트를 통과하면서 계약을 위반하는
형태이므로, 게이트 green은 이들의 부재를 증명하지 않는다. (구린내 캠페인에서 서로 다른 조각의
독립 리뷰가 같은 패턴을 각각 발견했다.)

1. **선언과 강제의 분리** — 스펙 YAML이 불변식을 선언하는데 그 필드를 읽는 런타임/DB 코드가 0건.
   *수색법*: 새로 추가된 안전 관련 필드명을 리포 전체 grep. 소비자가 스펙·테스트·문서뿐이면 死코드다.
   *실증*: 룰 5종의 "운영 BLOCKED"가 4가지 필드명으로 흩어졌고 기계 레지스트리엔 아예 없었다.
2. **직렬화 계약 미검증** — DB 가드가 조회하는 키와 실제 저장 shape이 다르다(camelCase↔snake_case 등).
   NULL 삼항논리로 `IF NULL THEN`이 **절대 발화하지 않고** CHECK는 통과한다.
   *수색법*: 새 가드의 예외 문자열을 리포 전체 grep. 마이그레이션 파일 밖에 없으면 **한 번도 실행된 적 없다.**
   *규칙*: **DB 가드를 추가하는 diff는 "위반 시 예외가 실제로 발생하는" 음성 테스트를 반드시 동반해야 한다.**
   음성 테스트가 없으면 그 가드는 없는 것으로 간주하고 blocker로 올린다.
3. **소극(集合) 검증 부재** — 개별 항목의 건전성(soundness)은 엄격히 재검증하면서
   집합의 완전성(completeness)은 호출자에게 위임한다. 빈 배열 제출로 PASS를 얻을 수 있다.
   *수색법*: `count = 0 → PASS` 형태를 찾고, 그 count의 출처가 요청 JSON인지 서버 계산인지 본다.
   *실증*: 실명 스캔이 `findings:[]`로 PASS를 얻었고, 스캔 대상 필드 목록 자체도 앱이 정했다.
4. **형제 불일치 fail-open** — 같은 파일의 형제 핸들러들은 `.ok_or(...)?`로 fail-close하는데
   하나만 `if let Some(...)` / `unwrap_or_default()`로 조용히 건너뛴다.
   *수색법*: 안전 필드를 다루는 `if let Some` / `unwrap_or` / NULL 비교를 찾아 **형제 코드와 대조**한다.
   차이가 있으면 의도인지 사고인지 묻지 말고 blocker다 — 의도라면 주석이 있었을 것이다.

공통 뿌리는 같다: **shape 검증을 상류에 위임하고 하류는 관대하게 처리**하는 습관이다.
"현재 writer가 그렇게 쓰니까 안전하다"는 구조적 보장이 아니다 — 그 문장이 리뷰에 등장하면 blocker다.

### 판정의 근거는 "조각"이 아니라 "유효 상태"다 (감독자 자신에게 적용)

리뷰가 지목한 파일·라인을 열어 보는 것은 **검증이 아니다.** 구린내 캠페인에서 감독자가 같은 부류의
오판을 두 번 했고, 둘 다 구현자가 잡아냈다:

1. **워크트리를 커밋으로 착각** — 리뷰의 "이 코드는 이미 있다"를 워크트리에서 확인했는데, 그것은
   다른 단위가 작업 중이던 미커밋 상태였다. 커밋된 ref에는 없었다.
   → 판정은 반드시 `git show <ref>:<path>`로 한다.
2. **교체되는 산출물의 첫 등장만 확인** — 리뷰가 "마이그레이션 0033의 CHECK에 값이 빠졌다"고 했고
   0033을 열어 확인했다. 그러나 같은 제약을 0028·0033·**0037**이 차례로 `DROP`+`ADD`했고 0037이
   그 값을 이미 포함하고 있었다. 결함은 존재하지 않았다.
   → `CREATE OR REPLACE`·`DROP`+`ADD CONSTRAINT`·override·후속 시드로 **교체 가능한 산출물**은
   파일 grep으로 판정하지 않는다. **순서대로 적용한 최종 유효 상태**를 확인한다
   (마이그레이션이면 실제 DB에 순차 적용 후 카탈로그 조회).

판정 전에 물어라: **이 사실은 어느 시점·어느 표현의 것인가?** 워크트리인가 커밋인가,
첫 정의인가 최종 유효 상태인가. 틀린 판정은 잘못된 지시가 되어 구현자에게 전파된다 —
실제로 그 오판으로 잘못된 steer를 보낸 적이 있다.

부수 효과 하나: 이런 오판이 반복되면 **그 자체가 게이트 부재의 신호**다. 위 2번의 경우
"세 마이그레이션이 같은 제약을 두고 다퉜는데 최종 허용집합을 아무도 파일만 보고는 알 수 없다"가
진짜 결함이었고, 수정은 값 하나가 아니라 **최종 유효 상태를 도출해 검증하는 게이트**였다.

## Phase 4 — PR 스택

단위가 완료되면(게이트 통과 + 감독자 검증) **전용 브랜치에 커밋하고 PR을 만든다.**

1. **브랜치 = 단위**. PR의 base는 **직전 단위 브랜치**다 (main 대상이 아니라 스택).
2. PR 본문에 그 단위의 목적 · 변경 규모(파일 수·줄수) · 게이트 결과 원문 · 남은 리스크를 적는다.
3. **PR 하나씩 리뷰**: 로컬 적대 리뷰 → 실질 blocker fix → ChatGPT Pro 구현 게이트 LGTM.
   **LGTM 전에는 다음 단위에 착수하지 않는다.**
4. 다음 단위는 시작 시 직전 브랜치로 **리베이스**한 뒤 진행한다. 앞 조각이 수정되면
   뒤 조각들도 리베이스 후 재검증한다.
5. **머지는 하지 않는다** — 스택만 쌓고 머지 판단은 사람이 한다.

스택이 길어지면 앞에서부터 LGTM을 받아 내려오는 것이 원칙이다. 뒤 조각의 문제를
앞 조각에서 고치지 않는다(그러면 앞의 LGTM이 무효가 된다).

**단, 마이그레이션·순서 있는 산출물은 예외다 — 수정이 앞으로만 쌓인다.**
DB 마이그레이션은 append-only이고 전역 순서가 있으므로, `0041`이 만든 결함을 `0041`을
담은 조각에서 고칠 수 없다. 반드시 **더 뒤 번호의 마이그레이션**으로만 고칠 수 있고,
따라서 그 수정 단위는 **스택 tip**에 놓아야 한다. "결함을 만든 조각에서 고친다"는 원칙을
기계적으로 적용해 앞 조각에 배치하면, 그 조각에는 번호를 매길 기준(직전 마이그레이션)이
없어 구현자가 착수조차 못 한다. 같은 제약이 append-only 원장·digest 사슬·시퀀스 번호를
쓰는 모든 산출물에 적용된다.

수정 단위를 만들기 전에 물어라: **이 결함을 고치는 산출물이 순서를 갖는가?**
그렇다면 배치는 tip이고, 원래 조각의 PR에는 "수정은 조각 N에서 이루어진다"고 기록한다.

### 이중 게이트 수렴 (필수 — 완료 선언 조건)

수렴은 두 게이트를 **순서대로, 둘 다 0건**이 될 때까지 돈다:

1. **sol 게이트**: 교차 모델(sol) 적대 리뷰 라운드를 BLOCKING 0건(CLEAN)까지 반복한다.
2. **Pro 게이트**: sol CLEAN 상태의 산출물을 홈서버 gpt-webai-pro로 보내 판정받는다.
   - Pro가 blocker를 내면 → Fable이 판정·반영하고 → **sol 회귀 검증 라운드**(반영이 새 결함을
     만들지 않았는지)를 CLEAN까지 돌린 뒤 → **다시 Pro에 보낸다**.
   - 이 사이클을 **Pro `<verdict>LGTM</verdict>`(BLOCKING 0건)까지 반복**한다.
3. Pro LGTM 없이 완료를 선언하지 않는다. sol CLEAN은 중간 상태일 뿐이다.
4. 라운드 간 재제기 방지: 매 Pro 라운드 프롬프트에 이전 라운드 blocker별 처리 결과(반영/기각+사유)를
   담고, 기각 항목의 동일 논거 재제기를 금지한다. 수렴·에스컬레이션 기준은 Phase 1.5와 동일
   (blocker 수가 2라운드 연속 미감소 또는 3라운드 내 LGTM 실패 시 사람에게 에스컬레이션).
5. Pro 왕복은 수 시간이 들 수 있으므로, Pro 대기 중에 다음 단위 sol 작업을 선진입하지 않는다
   (Pro가 뒤집을 수 있는 산출물에 시간을 태우지 않는다).

## 불변식

- **작업 단위는 짧게.** Phase 0.5 상한을 넘는 단위를 만들지 않는다.
- **1 단위 = 1 PR = 1 리뷰 = 1 LGTM.** LGTM 없이 다음 단위로 넘어가지 않는다.
- **이중 게이트**: sol 적대 루프 CLEAN → Pro 게이트, Pro blocker는 반영 후 sol 회귀 검증을 거쳐 다시 Pro로 — Pro 0건까지.
- 커밋/푸시/머지는 사용자가 명시적으로 요청할 때만. **PR 직접 머지는 절대 금지.**
- 검증 evidence(실제 실행한 명령과 결과) 없이 완료 선언 금지. 수행 못한 검증과 남은 리스크는 명시한다.
- 전역 `~/.codex/config.toml`을 이 스킬이 수정하지 않는다 — 모델/effort는 항상 CLI 플래그로 override.
- codex에 `--dangerously-bypass-approvals-and-sandbox` 같은 추가 위험 플래그를 붙이지 않는다 (전역 설정으로 충분).
- GPT 외부 검증(설계·구현 게이트, 기본 ON)은 **홈서버 `gpt-webai-pro` 슬롯 데몬만** 사용한다 (2026-09-01 갱신). chatgpt-pro-ask(맥 화면 자동화)·gptpro·gptxhigh·gpt-webai-lifecycle은 이 루프에서 사용 금지.
- `.fable-sol/`은 작업 산출물 디렉토리다. 리포지토리에 커밋 대상이 아니면 정리하거나 gitignore를 안내한다.

## 완료 보고 형식

1. 변경 파일 요약 (기획서 대비 커버리지)
2. 실행한 검증 명령 + 결과 (원문 evidence)
3. codex 세션 ID와 fix 라운드 횟수
4. ChatGPT Pro 리뷰 결과 — 설계/구현 게이트 각각의 라운드 수와 최종 verdict (`.fable-sol/review-log.md` 참조)
5. 남은 리스크 / 수행하지 못한 검증
