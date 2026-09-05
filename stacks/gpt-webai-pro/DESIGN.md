# gpt-webai-pro — 설계 (v2, from scratch)

ChatGPT 웹 위임 자동화의 2세대 구현. 일반 위임은 **Pro**, 이미지 배치는 **Xhigh**를 사용한다.
1세대(`stacks/gpt-webai-slot-pool`, PR #72)의 동작하는 개념(중복 전송 방지, send-reconcile,
시맨틱 셀렉터, 슬롯 풀)을 유지하되, 그 복잡도의 근원 두 가지 —
**(a) stateless per-exec 프로바이더가 만든 상태 증명(해시 바인딩) 문제**,
**(b) 구현자 불신이 만든 봉인 거버넌스** — 를 제거한다.

이 문서가 이 스택의 유일한 설계 문서다. 변경은 git 커밋/PR 리뷰로 관리한다.
MANIFEST 해시 봉인, 정본 바이트 동결, 결정 addendum 체인은 **의도적으로 없다**.

## 0. 목표 / 비범위

**목표**
- `gptpro "프롬프트"` (+ `--file` 첨부) → 실제 ChatGPT Pro Extended 세션에 전송 →
  답변 markdown + ChatGPT가 렌더한 다운로드 파일(artifacts)을 host에 저장 → JSON envelope로 반환.
- 절대 중복 전송하지 않는다 (전송 불확실 시 fail-closed + resume으로 해소).
- Pro Extended는 수 시간 걸릴 수 있다: timeout은 실패가 아니라 `running` + `resumeCommand`.
- 동시 요청 다수(여러 Claude 세션이 동시에 gptpro 호출)를 처리하되, **로그인 관리 단위는
  계정이다**: 슬롯 = 계정 = Chrome 프로필 1개 = 로그인 1개. 동시성은 슬롯을 늘리는 게
  아니라 한 Chrome 안의 **탭 멀티플렉싱**으로 확보한다 (§8.1). 프로필 복제로 로그인을
  "일괄화"하지 않는다 — ChatGPT의 rotating refresh token 때문에 같은 세션 사본 여럿이
  서로를 로그아웃시킨다.
- 로그인 수명주기 운영을 1급 기능으로: 원클릭 재시딩(`login`, §9.1) + 일일 keepalive(§9.2).
- UI 리디자인에 강한 시맨틱 셀렉터. 셀렉터는 한 파일에 모아 한 곳만 고치면 되게 한다.

**비범위 (구현 금지)**
- 일반 텍스트 요청의 임의 모델/Thinking 전환. 전용 `image-batch`만 Xhigh를 사용한다. (`gptxhigh` 실행기는 폐기 상태 유지)
- 이벤트 소싱, append-only 저널, projection, snapshot, HEAD CAS.
- content-addressed ID, 해시 파생 바인딩, canonical-JSON 바이트 비교, fencing token, dead-owner proof.
- Rust. 다른 언어 미러 계약. R12/R13 호환 레이어.
- 스크롤바 픽셀 증명, PNG 코덱, 3중 증인 방식의 검증.
- 멀티 호스트, MCP 서버, "미래의 다른 프로바이더"를 위한 추상화.

## 1. 아키텍처 개요

```
gptpro (thin bash) ──▶ gpt-webai-pro CLI (TypeScript, 단명 프로세스)
                          │  SQLite (state root, 유일한 진실)
                          │  docker run/stop (슬롯 컨테이너 on-demand)
                          ▼  WS JSON-RPC over TCP 127.0.0.1:<슬롯포트> (+bearer 토큰)
                    slot container (gwp-slot-NN)
                      ├─ Xvfb :99
                      ├─ Chromium (CDP 127.0.0.1:9222, 영속 프로필)
                      └─ slot-daemon (장수 Node 프로세스)
                           └─ Playwright Page 객체를 메모리에 유지
```

핵심 원칙:

1. **daemon이 상태를 들고 있다.** 한 요청의 모델 확인→첨부→전송→턴 확인은 daemon 프로세스
   안에서 같은 `Page` 객체로 일어난다. 프로세스 간 "같은 페이지인가"를 해시로 증명할 필요가
   원천적으로 없다. v1의 rootBindingHash/PageBindingEcho에 해당하는 것은 **아무것도 없다**.
2. **supervisor(CLI)가 유일한 기록자다.** daemon은 브라우저 사실을 관찰·수행해 RPC 응답으로
   돌려줄 뿐, 디스크에 진실을 쓰지 않는다 (evidence 스크린샷 제외). SQLite는 CLI만 쓴다.
3. **작게.** 런타임 의존성은 playwright-core, better-sqlite3, ws, sharp다. 목표 규모
   전체 8k 라인 이하 (테스트 포함).

## 2. 디렉토리 레이아웃

```
stacks/gpt-webai-pro/
  DESIGN.md                  # 이 문서
  README.md                  # 운영 런북 (설치, 로그인 시딩, 트러블슈팅)
  package.json  tsconfig.json  .gitignore
  bin/gpt-webai-pro          # bash shim: exec node <root>/dist/cli/main.js "$@"
  config/
    slots.json               # 슬롯 정의 (아래 §8)
    labels.json              # 모델 피커 라벨 세트 (아래 §6.3)
  src/
    cli/main.ts              # argv 파싱 + 커맨드 디스패치
    cli/envelope.ts          # 공개 JSON envelope (아래 §9)
    supervisor/db.ts         # SQLite 스키마 + 쿼리 (아래 §4)
    supervisor/run.ts        # run/resume 오케스트레이션 + 전송 멱등성 (아래 §5)
    supervisor/slots.ts      # 슬롯 할당, 쿨다운, 계정 로테이션
    supervisor/docker.ts     # docker CLI 호출 (create/start/stop/inspect)
    supervisor/rpc-client.ts # loopback TCP WS JSON-RPC + bearer 클라이언트
    daemon/main.ts           # WS 서버 + RPC 디스패치 (컨테이너 안에서 실행)
    daemon/browser.ts        # CDP 접속, Page/탭 관리
    daemon/selectors.ts      # 모든 DOM 셀렉터 + 라벨 매칭 (단일 파일, §6)
    daemon/actions/model.ts  #   기본 Pro / 이미지 Xhigh 보장
    daemon/actions/send.ts   #   첨부 + 전송 + 턴 시작 확인
    daemon/actions/poll.ts   #   생성 완료 감시 + 답변 추출
    daemon/actions/download.ts # artifact 컨트롤 발견 + 다운로드
    daemon/actions/reconcile.ts # 전송 불확실 복구 (§5.3)
    shared/types.ts  shared/errors.ts  shared/fsx.ts  shared/ids.ts
  container/
    Dockerfile               # node:24-bookworm-slim + chromium + xvfb + dist
    entrypoint.sh            # xvfb → chromium(CDP) → daemon
  test/
    unit/*.test.ts           # db, 멱등성 상태기계, envelope, slots
    fake-chatgpt/            # ChatGPT 모사 하네스 (§11.2)
    daemon.e2e.test.ts       # daemon ↔ 실제 chromium ↔ fake-chatgpt
    supervisor.e2e.test.ts   # supervisor ↔ mock daemon (전송 멱등성 시나리오)
  scripts/
    container-smoke.sh       # 이미지 빌드 + 컨테이너 1개로 fake 하네스 왕복
  systemd/
    gpt-webai-pro-keepalive.service  # user unit (§9.2)
    gpt-webai-pro-keepalive.timer
```

state root: `${GPT_WEBAI_PRO_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-pro}`

```
<state>/db.sqlite
<state>/requests/<reqId>/prompt.md
<state>/requests/<reqId>/attachments/<files>            # run 시작 시 원본 복사 (진실; inbox는 사본)
<state>/requests/<reqId>/answer.md
<state>/requests/<reqId>/artifacts/<filename>
<state>/requests/<reqId>/failure/*.png|*.html      # 실패 시에만
<state>/requests/<reqId>/log.jsonl                 # 상태 전이 append 로그 (사람용, 진실 아님)
<state>/slots/<slotId>/profile/                    # Chrome 프로필 (영속)
<state>/slots/<slotId>/daemon.token               # RPC bearer 토큰 (0600, supervisor 생성)
<state>/slots/<slotId>/inbox/<reqId>/<files>       # 첨부 스테이징 (ro mount)
<state>/slots/<slotId>/outbox/                     # daemon 다운로드 착지 (rw mount)
```

## 3. 컴포넌트 책임

| 컴포넌트 | 책임 | 하지 않는 것 |
| --- | --- | --- |
| `gptpro` wrapper | argv/stdin 정규화 후 CLI exec | 로직 없음 |
| CLI (supervisor) | 요청 수명주기, SQLite 기록, 슬롯 할당, 컨테이너 기동/정지, RPC 호출, envelope 출력 | DOM 접촉 |
| slot-daemon | 브라우저 조작 전부, 관찰 결과 반환, 실패 시 evidence 캡처 | SQLite 접근, 수명주기 판단 |
| 컨테이너 | 격리된 Chrome 런타임 | 그 외 전부 |

## 4. 데이터 모델 (SQLite)

### 4.0 주간 사용량 원장 `usage_events` (user_version 3, 2026-09-05)

```sql
CREATE TABLE usage_events (
  request_id  TEXT PRIMARY KEY REFERENCES requests(id),
  slot_id     TEXT NOT NULL REFERENCES slots(id),
  model_label TEXT,            -- send 결과의 modelLabel (예: '6 Pro'), reconcile 확정은 NULL
  sent_at     INTEGER NOT NULL
);
CREATE INDEX usage_events_slot_sent ON usage_events (slot_id, sent_at);
```

- 기록 시점: `confirmSendAttempt`(confirmed)와 `applyReconcileResult`(reconciled) — 전송이 확정된 순간
  요청당 1건(`INSERT OR IGNORE`). 이미 확정된 요청의 재확정·resume은 계상하지 않는다.
- 집계: `weeklyUsageFor(slot)` = `sent_at >= now − 7d` 건수. 한도는 `slots.json`의 `weeklyLimit`(공통) /
  `slot.weeklyLimit`(개별), 없으면 무제한. 한도에 닿은 슬롯은 `selectSlot`에서 제외되고, 상태로는
  쓸 수 있는 슬롯이 전부 한도에 닿으면 envelope `recovering`/`weekly_limit`(+가장 이른 리셋 시각).
- v2→v3 이관은 표 추가만 한다. 과거 전송은 증거가 없으므로 소급 계상하지 않는다.

better-sqlite3, `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;`
동시 CLI 호출 간 DB 배타는 SQLite 트랜잭션(`BEGIN IMMEDIATE`)으로 해결한다.
락 파일은 모두 kernel flock 생존 증명이다. 요청별 `send.lock`은 §5.2의 전송 임계구역,
`owner.lock`은 한 요청의 전체 run/resume 호출, 슬롯별 `runtime-control.lock`은 Docker
start/stop 교차, `runtime-activity.lock`은 login/keepalive/cleanup 유지보수 호출을 보호한다.
프로세스가 죽으면 커널이 자동 해제하므로 PID/TTL 파일은 두지 않는다.

```sql
CREATE TABLE requests (
  id            TEXT PRIMARY KEY,          -- 'req_' + 16 lower-hex (crypto random)
  prompt_sha256 TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN
                ('staged','sending','generating','complete',
                 'uncertain','needs_user_action','failed')),
  slot_id       TEXT,
  conversation_url TEXT,                   -- https://chatgpt.com/c/... (non-root만 저장)
  answer_sha256 TEXT,
  error_kind    TEXT, error_detail TEXT,
  created_at    INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE send_attempts (
  request_id  TEXT NOT NULL REFERENCES requests(id),
  attempt_no  INTEGER NOT NULL,            -- 1 또는 2. 3 이상 금지.
  state       TEXT NOT NULL CHECK (state IN
              ('armed','confirmed','reconciled','no_send_proven','uncertain')),
  user_turn_id TEXT, assistant_turn_id TEXT,   -- ChatGPT의 data-message-id 원문
  created_at  INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, attempt_no)
);
CREATE TABLE artifacts (
  request_id TEXT NOT NULL REFERENCES requests(id),
  filename   TEXT NOT NULL, path TEXT NOT NULL,
  sha256     TEXT NOT NULL, size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, filename)
);
CREATE TABLE slots (
  id             TEXT PRIMARY KEY,          -- config/slots.json과 조인
  account        TEXT NOT NULL,
  state          TEXT NOT NULL DEFAULT 'idle' CHECK (state IN
                 ('idle','needs_login','provider_limit')),
  cooldown_until INTEGER, last_used_at INTEGER
);
```

`busy`는 슬롯 상태가 아니다 — 슬롯 점유도는 requests 테이블의 비종결 행 수로 파생한다
(`user_version=2` 마이그레이션: slots 테이블 재생성, 기존 'busy' 행은 'idle'로).
config 동기화 시 config에 없는 슬롯 행은 비종결 요청이 참조하지 않으면 삭제한다
(과거 requests.slot_id 문자열은 이력으로 그대로 둔다).

`log.jsonl`은 디버깅 편의용 append 로그이고 진실이 아니다. 복구는 항상 SQLite 기준.

### 4.1 이미지 배치 (user_version 4)

`image_batches(id, created_at)`와 `image_chunks(batch_id, ordinal, request_id, items_json)`를
추가한다. 청크당 1–5개 ID/프롬프트를 저장하고 `(batch_id, ordinal)` 및 `request_id`가
유일하다. 입력 프롬프트와 첨부를 요청 디렉터리에 먼저 복사한 뒤 배치와 모든 child request를
한 SQLite transaction으로 생성한다. 미완성 입력은 staged로 노출되지 않는다.
v3→v4는 새 두 표만 추가하고 기존 요청·전송·사용량을 보존한다.

배치 `owner.lock`은 순차 실행을 직렬화하며 각 child는 기존 요청 `owner.lock`, `send.lock`,
`send_attempts`를 그대로 사용한다. 배치 전체가 하나의 제한 시간을 공유한다. 시간이 끝나면
다음 청크를 보내지 않는다. 재개는 저장된 child ID만 사용하고 완료한 청크를 재전송하지 않는다.
이미지 다운로드의 재시도 제한은 supervisor finalization 호출당 2회다. daemon 수명 전체의
재시도 횟수 제한을 이미지에 적용하면 같은 응답의 다운로드 재개가 막히므로 적용하지 않는다.
한 이미지가 두 번 실패하면 해당 청크의 수집을 멈추고 재개를 요구한다. 저장된 원본은
유지하고 미저장 이미지부터 다시 받는다.

## 5. 요청 수명주기와 전송 멱등성 (시스템의 심장)

### 5.1 run 흐름

```
staged ─▶ sending ─▶ generating ─▶ complete
   │          │            │
   │          ├─▶ uncertain (전송 여부 미증명, fail-closed)
   │          │
   └──────────┴─▶ needs_user_action | failed
```

1. `run`: 프롬프트/첨부 검증 → `requests` 행 삽입(staged) → prompt.md +
   `requests/<id>/attachments/`에 첨부 복사 (슬롯 할당 전 요청 레벨 영속화 — pool-busy 후
   resume에도 안전. 별도 DDL 불필요, 이 디렉토리가 곧 manifest).
   동일 basename 충돌은 첫 `.` 앞에 `-2`,`-3` 부가로 해소(복합 확장자 보존:
   `a.tar.gz`→`a-2.tar.gz`). 이후 RPC `files[].name`은 이 최종 이름.
   슬롯 할당 시점에 `slots/<slotId>/inbox/<reqId>/`로 복사한다.
2. 슬롯 할당(§8): `BEGIN IMMEDIATE` 트랜잭션 안에서 "할당 가능(§8) AND 비종결 요청 수 <
   maxConcurrent"인 슬롯 중 **점유도 최소 → last_used_at 최구(最舊)** 순으로 선택해
   requests.slot_id를 기록한다(이 기록이 곧 점유). 없으면 envelope `recovering`
   + `nextCommand: resume`(대기 재시도는 호출자 몫; 큐잉 데몬은 만들지 않는다).
3. 컨테이너 보장: 안 돌면 이전 stopped 컨테이너를 교체하고 fresh token으로 create/start,
   authenticated health 준비 대기(최대 60s).
4. `daemon.readiness()` → `needs_login`/`provider_limit`이면 슬롯 상태 기록 후 다음 슬롯 시도.
   전 계정 소진이면 envelope `recovering`(provider_limit) 또는 `needs_user_action`(login).
5. **전송 (멱등성 프로토콜 §5.2)** → 성공 시 conversation_url + 턴 id 기록, status=generating.
6. poll 루프(§7): 완료 시 answer.md 저장 → artifacts 다운로드 → status=complete → envelope.
   timeout 도달 시 status=generating 유지, envelope `running` + resumeCommand.
7. 종료 처리: CLI 호출의 `owner.lock`을 놓기 전에 같은 슬롯에 살아 있는 다른 owner가
   없으면 `docker stop`. 요청은 `generating` 등 비종결 상태와 conversation URL을 그대로
   보존하며, 다음 resume이 profile을 사용해 runtime을 다시 기동한다.

### 5.2 전송 멱등성 프로토콜

**불변식: ChatGPT에 같은 요청이 두 번 전송되는 일은 없다.** 이를 위해 intent를 클릭 전에
기록하고, 결과가 불확실하면 절대 재클릭하지 않는다.

```
supervisor                                daemon
─────────                                 ──────
INSERT send_attempts (attempt_no=1, armed)
        ── rpc send({prompt, files}) ──▶  모델 보장 → 첨부 → 칩 검증
                                          → [클릭] → 새 user+assistant 턴 확인
        ◀── ok {conversationUrl, turnIds}
UPDATE armed→confirmed, status=generating
```

**전송 임계구역 (동시성 규칙)**: sender는 attempt 삽입 직전에
`requests/<id>/send.lock` 파일에 **flock(EX, non-block)**을 잡고, [armed 삽입 → send RPC →
최종 DB 갱신] 전 구간 동안 유지한다. 이 flock이 곧 생존 증명이다 — 프로세스가 죽으면
커널이 자동 해제하므로 PID 추적/하트비트/TTL이 필요 없다. 같은 요청을 만지려는 다른
프로세스(동시 resume 등)는 같은 flock을 non-block으로 시도해서:
- **실패(다른 프로세스 생존)** → DB/daemon을 일절 변경하지 않고 envelope
  `status:"running"`, message "전송 진행 중(소유 프로세스 생존)" 반환.
- **성공(소유자 사망 확정)** → flock을 쥔 채 복구 진행: guarded update로 armed→uncertain
  후 §5.3 reconcile.

DB의 모든 attempt 상태 전이는 **guarded UPDATE**(`... WHERE state IN (<허용 출발 상태>)`)로
수행한다. 0행 갱신이면 재조회해서 이미 도달한 종결 상태(confirmed/reconciled)를 그대로
수용한다 — confirmed/reconciled를 덮어쓰는 전이는 존재하지 않는다. attempt 2 arm은
"직전 attempt가 no_send_proven일 때만" 조건을 같은 트랜잭션 안에서 검사해 원자 삽입한다.

**전송 확정 매칭 규칙 (2026-07-28 라이브 수정 — 오탐이 중복 전송을 유발했음)**: 새 user
턴은 `turn.text === prompt` **정확 일치로 판정하지 않는다**. 첨부가 있으면 렌더된 user 턴
텍스트 앞에 첨부 파일명/라벨이 붙어(`bundle.tar.gz File … <prompt>`) 절대 같지 않고,
코드펜스/유니코드도 innerText가 원문과 달라진다. 판정 프리미티브
`renderedTurnMatchesPrompt(rendered, prompt)` (send·reconcile 공용, 단일 헬퍼):
- 양쪽을 **마크다운 무해 정규화**한다: CRLF→LF, 코드펜스 라인(정확히 ``` 또는 ```lang)
  제거, 행 앞뒤 공백 제거, 연속 개행 축약, trim.
- ChatGPT가 fenced code의 언어를 별도 헤더/배지 줄로 렌더하면, 원문 opening fence의 언어와
  같은 위치·같은 토큰인 variant만 추가로 허용한다. `python` 같은 언어 단어를 전역 제거하지 않는다.
- 렌더 텍스트에서 **선행 첨부 라벨 블록을 제거**한 본문이 정규화 프롬프트와 같거나
  그것을 `endsWith` 하면 매칭. (임의 "마지막 N자" 규칙은 쓰지 않는다 — 취약하고 오인
  위험. 코드펜스 제거로 파이썬/유니코드 케이스가 결정적으로 매칭된다.)
- 확정 루프(send.ts)는 우리가 방금 클릭한 **그 탭**에서만 보므로, "baseline에 없던 새 user
  턴 + 이 매칭"이면 충분(같은 탭에 우리 외 발신자 없음). 이 매칭 실패로 인한 확정 창
  초과는 실제 전송 성공 오탐이므로 절대 `no_send_proven`으로 떨어지면 안 된다(아래).

**클릭 시점 durable 앵커 (중복 방지의 핵심)**: 안전성은 텍스트가 아니라 **ChatGPT가 서버에서
부여하는 user turn id**(전역 유일)로 보장한다. daemon `send`는 클릭 직후 확정 루프에서
이 탭에 나타난 **첫 새 user 턴 id**를 잡는 즉시, 확정(assistant/URL)이 나중에 실패하더라도
그 값을 확보한다. 확정 실패로 throw할 때 에러에 `pendingUserTurnId`(잡았으면),
`pendingConversationUrl`(그 탭 현재 URL, 루트여도), `preClickBaseline`을 싣는다.
supervisor는 이를 **클릭이 실제로 일어났다는 증거**로 DB에 즉시 기록한다
(`send_attempts.user_turn_id`에 pendingUserTurnId, requests.conversation_url이 비면
pendingConversationUrl). turn id는 DDL/envelope 신설이 아니라 기존 필드 활용이다.

실패 분기 — daemon의 모든 send 에러는 `phase: 'pre_click' | 'post_click'`을 반드시 포함하고,
**pre_click은 daemon이 "send 버튼이 눌리지 않았음"을 적극 확인했을 때만** 부여한다
(클릭 시도 자체가 시작됐으면 무조건 post_click):

- **pre_click 에러** (모델 라벨 부재, 칩 불일치, 컴포저 접근 실패 — 클릭 코드에 진입 전):
  전송이 발생하지 않았음이 확실. attempt → `no_send_proven`. needs_user_action/failed 또는
  슬롯 교체 재시도(§8). 단 재시도(attempt 2)는 §5.3의 **긍정적 부재 증명**을 통과해야만
  실제 재전송한다 (오분류 방어).
- **post_click 에러 / 확정 창 초과 / 클릭 예외 / RPC 타임아웃 / 소켓 단절 / daemon 사망**:
  attempt → `uncertain`, request → `uncertain`, `pendingConversationUrl` 기록.
  **여기서 절대 즉시 재전송하지 않는다.** 복구는 §5.3.

**send RPC 대기 규칙 (2026-07-29 재설계 — 고정 타임아웃이 앵커를 유실시켰음)**: supervisor는
send RPC에 고정 짧은 타임아웃을 걸지 않는다. daemon은 send 진행 중 단계 전이·주기(2.5s)
heartbeat를 JSON-RPC notification(`gwp.sendProgress`)으로 흘리고, 관측 즉시
`pendingUserTurnId`/`pendingConversationUrl`/`preClickBaseline`을 알림에 복제한다.
supervisor는:
- **무진행 상한** `GWP_SEND_INACTIVITY_MS`(기본 120s): 알림이 이 간격 안에 계속 오는 한
  기다린다. 끊기면 daemon 사망/행으로 보고 포기한다.
- **절대 상한** `GWP_SEND_MAX_MS`(기본 30분): 어떤 경우에도 이 이상 기다리지 않는다.
- **포기하더라도 마지막 progress의 앵커를 그대로 `markSendUncertain`에 싣는다**
  (phase는 무조건 post_click — 클릭 가능성을 배제할 수 없으므로 fail-closed).
  과거 "RPC send timed out"이 앵커 0개로 떨어져 reconcile이 최악 경로(텍스트 탭 스캔)로
  가던 구멍(2026-07-29, 94KB 프롬프트 180s 초과 사건)이 이 규칙으로 막힌다.
- daemon은 supervisor가 포기한 뒤에도 send를 완주하고, 성공 결과를 promptSha 키로
  메모리에 캐시한다(§5.3 A0). 정상 완료 시 확정 창 초과가 아니라 회수 가능한 진실이 된다.

### 5.3 uncertain 복구 (reconcile)

`resume --session <id>`가 uncertain 요청(또는 §5.2 flock 획득으로 사망이 확정된
sending/armed 요청)을 만나면 — 반드시 해당 요청의 `send.lock` flock을 쥔 상태로:

1. 슬롯 컨테이너/daemon 재기동(필요 시).
2. `daemon.reconcile({prompt, promptSha256, conversationUrl?, pendingConversationUrl?,
   pendingUserTurnId?})`. reconcile에는 **원문 `prompt`를 전달**해 §5.2 공용 헬퍼
   `renderedTurnMatchesPrompt`를 그대로 쓴다(promptSha256은 로그/보조용으로 유지). 매칭
   우선순위:
   - **(A0) daemon 메모리 send 캐시**: 같은 daemon 프로세스에서 이 promptSha의 send가
     이미 성공 완료했다면 그 결과(`conversationUrl`/turn id)를 그대로 회수한다. mutation
     큐가 순서를 보장하므로(진행 중 send가 끝난 뒤에야 reconcile 클로저가 돈다) 캐시는
     항상 완결된 진실이다. supervisor가 대기를 포기한 뒤 daemon이 완주한 케이스의
     결정적 회수 경로.
   - **(A) `pendingUserTurnId`가 있으면 그 turn id를 전역에서 찾는다** — 있으면 그 탭이 곧
     내 요청. 전역 유일 id라 오매칭 불가. 이게 1순위이자 모호성의 근본 해소책.
   - (B) 없으면 `conversationUrl`/`pendingConversationUrl` 탭을 확인(텍스트 매칭).
     URL 앵커는 우리 DB에 기록된 우리 대화이므로, 유일한 user 턴에 한해 loose 매칭과
     single_turn 길이 sanity(§6.4 tier ③, 대화의 user 턴이 정확히 1개일 때)를 허용한다.
     매칭 실패 시 길이/first-diff evidence를 결과에 실어 supervisor 로그로 남긴다.
   - (C) 여전히 모르면 **열려 있는 chatgpt.com 탭들만** 스캔(텍스트 매칭). 사이드바/히스토리
     클릭 탐색은 하지 않는다. **여기서는 loose 매칭을 쓰지 않는다** — 앵커 없는 텍스트
     스캔에서 loose는 identity 증명이 아니다.
   - **양성 회수(found)는 부재 증명 권한과 분리한다.** 앵커가 없어도 유일한 bound `/c/`
     탭에서 user 턴이 매칭되고 다중 매치나 같은 프롬프트의 unbound(root) 매치가 없으면
     found로 회수한다. 다른 unreadable 탭은 이 확실한 양성 매치를 무효화하지 않는다.
     다중/unbound 매치는 모호하므로 보류한다. `canProveAbsence`는 found가 아니라 아래의
     부재 증명과 재전송 승인에만 적용한다.
3. 결과:
   - 턴 발견 → attempt `uncertain→reconciled`, status=generating, poll 계속. **재클릭 0회.**
   - **부재 증명(재전송 허용)은 매우 보수적으로**: 대화/탭 접근에 성공했고 그 안에 매칭
     user 턴/앵커가 확실히 없을 때만 `no_send_proven`. pendingUserTurnId 또는
     pendingConversationUrl이 있는데 해당 앵커/탭에 접근 못 하면 부재로 판정하지 않는다
     (uncertain 유지). **baseline 밖 새 user 턴이 매칭 없이 존재하면(어느 탭이든) 부재로
     판정하지 않는다** — 대형 프롬프트 렌더 변형으로 매칭만 실패한 자기 전송을 부재로
     오판하면 그대로 중복 전송이 된다 (2026-07-29 규칙 강화). **앵커된 `/c/` 페이지가
     턴 0개로 읽히면 unreadable로 취급한다(부재 증명 불가)** — 무거운 대화는
     domcontentloaded 후 턴 렌더까지 수 초가 걸려, 렌더 전 스캔이 빈 대화로 읽힌다.
     reconcile은 앵커 페이지에서 턴이 나타날 때까지 대기(`GWP_RECONCILE_RENDER_WAIT_MS`,
     기본 20s)한 뒤에만 판정한다 (2026-07-29 라이브: 렌더 전 스캔이 부재를 오판해
     attempt 2 중복 전송 발생). attempt_no<2 AND 긍정적
     부재 증명일 때만 새 attempt로 재전송.
     **앵커 없이 텍스트 매칭만으로 판정하는데 동일 프롬프트 탭이 하나라도 있으면(단일이든
     복수든) 재전송 금지** — anchor가 소실된 오매칭도 중복을 낳으므로 fail-closed. 중복보다
     uncertain이 낫다.
   - 증명 불가(Chrome 사망·탭 소실·앵커 소실·모호) → uncertain 유지. 호출 예산이 충분하면
     같은 owner가 read-only reconcile을 기본 최대 3회 추가 시도하며 간격은 20초다.
     각 대기 전 간격의 두 배 이상이 남아 있어야 하고, 대기 후 예산을 다시 확인한다.
     `GWP_INLINE_RECONCILE_TRIES`는 유한한 음이 아닌 정수만 허용하며 0은 비활성,
     잘못된 값은 기본 3회다. `GWP_INLINE_RECONCILE_BACKOFF_MS`로 간격을 설정한다.
     성공 시 기존 confirm/reconcile 트랜잭션에서만 사용량을 한 번 기록한다. 횟수·예산
     소진 시 needs_user_action envelope와 같은 세션의 resumeCommand를 반환한다.
     추가 관측은 부재 증명이 아니며 재전송·원장 계상 조건을 바꾸지 않는다.

### 5.4 resume 일반 규칙

`resume`은 상태 기반 멱등 재진입이다: staged→전송부터, uncertain→reconcile,
generating→poll 재개, complete→저장된 envelope 재출력. 어떤 상태에서 몇 번 불러도 안전.

`run`/`resume`은 먼저 `owner.lock`을 시도한다. 다른 owner가 살아 있으면 DB·daemon·runtime을
바꾸지 않고 결과를 기다린다. 기본 확인 간격은 `GWP_OWNER_ATTACH_POLL_MS=2000`이며 마지막
대기는 남은 예산 이내로 제한한다. 완료 관측 시 저장된 envelope를 반환하고, lock이 풀리면
남은 예산으로 처리를 이어받는다. 대기 예산 소진 뒤에는 lock을 다시 시도하지 않고
`running`을 반환한다. 최초 timeout=0 호출의 즉시 lock 획득 시도는 한 번 허용한다.
대기 예산과 이후 처리 예산은 하나의 절대 deadline을 공유하며, lock을 얻지 않은 호출은
runtime cleanup을 하지 않는다. `release`와 내부 `send.lock`의 non-block 규칙은 유지한다.

**reap — 방치 요청 자동 전진 (2026-07-30, 2026-08-15 운영 수정)**: 비종결 상태는
supervisor 프로세스가 돌 때만 전진한다 — 소유 세션이 running envelope 후 resume을
재실행하지 않으면 generating이 무한 방치된다. `gpt-webai-pro reap
[--timeout-seconds N=120]`은 **한 번에 sending/generating/uncertain 중 하나만** resume한다.
`updated_at → created_at → id` 순으로 고른 뒤 비종결이면 `updated_at`을 현재 시각으로
옮기므로 다음 tick은 다른 요청으로 공정 순회한다. 이 one-candidate 규칙이 timer 한 번의
전역 실행 예산이며, 요청별 max-age나 자동 실패 정책은 두지 않는다. **staged는 절대
건드리지 않는다** — 전송이 arm되지 않은 요청의 개시는 소유 세션의 결정이다.

전체 run/resume의 `owner.lock`이 살아 있는 호출과의 경합을 막는다. reap이나 일반 resume이
`running`을 반환해 owner가 끝나면, 같은 슬롯에 다른 live owner가 없는 managed runtime은
즉시 정지한다. ChatGPT 서버의 생성, SQLite 요청, conversation URL, 영속 profile은 보존되어
다음 tick/resume에서 이어간다. reaper는 마지막에 owner 없는 과거 runtime도 회수한다.
systemd timer는 부팅 5분 후 시작하고 **서비스가 끝난 뒤** 10분을 쉬며, 서비스 자체는
5분 상한을 갖는다.

## 6. 브라우저 조작 계약 (daemon)

### 6.1 셀렉터 원칙

- 전 셀렉터는 `daemon/selectors.ts` 한 파일에 상수/함수로 모은다. UI 변경 대응 = 이 파일 수정.
- 우선순위: `data-testid` > `role`/`aria-*` > 구조적 패턴(예: 파일명 토큰) > 텍스트 라벨.
  **CSS 클래스명·좌표·boundingBox를 식별에 쓰지 않는다.**
- v1에서 검증된 셀렉터 지식을 이식한다 (구현 시 원본 참조):
  - 컴포저: `#prompt-textarea`, `[contenteditable][role="textbox"]`
    (v1 `provider/chatgpt-playwright/lib/browser-composer.mjs`)
  - 턴/답변: `[data-message-author-role]` + `data-message-id`. **답변 텍스트와 턴 id는 반드시
    같은 DOM 노드에서 파생** (v1 hydration 버그의 교훈, `lib/session-rebind.mjs`)
  - 생성 중 판정: accessible name `/stop generating|stop responding|중지|정지/i` 버튼 존재
  - 첨부 칩: composer form 스코프에서 파일명 토큰 정규식
    `/[^\s:/\\"'<>|]+\.[a-z0-9]{1,8}\b/iu` + remove 버튼 accessible name 앵커,
    action 라벨(remove|delete|attach|…) 요소는 시드에서 제외 (v1 `lib/commands/upload-only.mjs`
    최종본 — 이 로직은 라이브 검증 완료본이므로 그대로 이식)
  - 모델 피커: `[data-testid="model-switcher-dropdown-button"]` 계열 + 라벨 매칭

### 6.2 RPC 프로토콜

JSON-RPC 2.0 over WebSocket over **TCP 127.0.0.1:<슬롯포트>** (슬롯별 고정 포트,
`slots.json`의 `port`; 컨테이너는 `-p 127.0.0.1:<port>:<port>`로 loopback에만 publish).

> **unix socket을 쓰지 않는 이유 (이 호스트의 실측 제약, 2026-07-27)**: 이 호스트의
> Claude Code 세션(= gptpro의 주 호출자)은 AppArmor `unprivileged_userns` 프로파일로
> confine되며, 커널 7.0의 unix socket peer 중재가 컨테이너 리스너로의 connect를
> EACCES로 거부한다 (권한/uid 완벽 일치·apparmor=unconfined 컨테이너에서도 재현).
> TCP loopback은 동일 조건에서 정상 동작 확인(codex-lb와 같은 패턴).

loopback publish는 로컬 모든 프로세스에 노출되므로 **bearer 토큰**으로 보강한다:
supervisor가 슬롯당 32-hex 토큰을 `slots/<slotId>/daemon.token`(0600)에 생성·보관,
컨테이너에 env `GWP_DAEMON_TOKEN`으로 전달, daemon은 WS 핸드셰이크의
`Authorization: Bearer <token>` 불일치/부재 시 401로 거부한다. 컨테이너 기동마다
토큰을 재생성한다(고정 토큰 금지). daemon ready 신호는 소켓 파일이 아니라
"TCP 연결 + `health` RPC ok"다.

| method | params | result | 비고 |
| --- | --- | --- | --- |
| `health` | – | `{ok, chromeConnected, currentUrl}` | |
| `readiness` | – | `{state:'ready'\|'needs_login'\|'provider_limit'\|'unknown', modelLabel}` | 로그인 UI/rate-limit UI 감지 |
| `send` | `{prompt, files:[{name,containerPath}]}` | `{conversationUrl, userTurnId, assistantTurnId}` | §5.2. 내부에서 model 보장+첨부+클릭+턴확인 |
| `reconcile` | `{promptSha256, conversationUrl?}` | `{found, conversationUrl?, userTurnId?, assistantTurnId?, proven:boolean}` | §5.3. 절대 클릭하지 않는 읽기 전용 |
| `poll` | `{conversationUrl, promptSha256, userTurnId?, assistantTurnId?, waitMs≤60000, imageCount?}` | `{state:'generating'\|'complete', currentUrl, assistantTurnId?, answerMarkdown?, answerSha256?, artifactControls?:[{index,label}]}` | identity/URL §6.5, 완료 판정 §7 |
| `download` | `{conversationUrl, controlIndex, assistantTurnId?, imageCount?, userTurnId?}` | `{filename, outboxPath, sha256, sizeBytes}` | 컨트롤당 1회, outbox에 저장 |

**daemon 동시성 규칙 (탭 멀티플렉싱)**: 브라우저를 **변경**하는 RPC(send, reconcile의
네비게이션 구간, download, closeConversation)는 단일 **mutation 큐**로 직렬
처리한다 — reconcile 판정이 진행 중 send와 인터리빙되지 않는 보장은 유지된다.
읽기 전용 RPC(poll의 관찰 루프, readiness, health)는 큐 밖에서 동시
실행한다(서로 다른 탭에 대한 동시 poll이 서로를 막지 않아야 한다). 단 poll이 탭
바인딩을 위해 네비게이션이 필요해지면 그 네비게이션만 mutation 큐를 거친다.

- **send는 항상 새 탭에서 시작한다** (`context.newPage()` → 루트 이동 → 컴포저).
  진행 중인 다른 요청의 탭을 건드리지 않는다.
- `closeConversation {conversationUrl}` RPC: 요청 종결 후 supervisor가 호출, 해당 탭을
  best-effort로 닫는다 (탭 누적 방지).
- entrypoint의 Chromium에 백그라운드 탭 스로틀링 방지 플래그를 준다:
  `--disable-background-timer-throttling --disable-backgrounding-occluded-windows
  --disable-renderer-backgrounding`.

에러: JSON-RPC error, `data: {kind, phase?, detail}`.
kind 폐쇄 목록: `needs_login, provider_limit, model_unavailable, nav_failed, compose_failed,
chip_mismatch, click_uncertain, turn_not_found, artifact_failed, internal`.

**progress notification**: send 진행 중 daemon은 같은 소켓으로 JSON-RPC notification
`{method:"gwp.sendProgress", params:{callId, progress}}`를 흘린다 (단계 전이 시 +
2.5s heartbeat). `progress = {step, phase, elapsedMs, stepElapsedMs, pendingUserTurnId?,
pendingConversationUrl?, preClickBaseline?, matchDebug?}`,
`step ∈ navigate|ensure_model|compose|attach|verify_chips|baseline|wait_send_button|click|confirm`.
supervisor `RpcClient.call`은 옵션 `{timeoutMs, inactivityMs, onProgress}`로 이를 소비한다
(§5.2 send RPC 대기 규칙).

**daemon 운영 로그**: daemon은 stderr(=docker logs)에 JSON 라인을 남긴다 — RPC 시작/종료
(`rpc`: method/ms/ok/kind), send 단계 전이(`send_step`), reconcile 판정(`reconcile`,
`reconcile_cache_hit`). 2026-07-29 이전엔 daemon이 무로그라 스톨 지점을 밖에서 알 수
없었다.

### 6.3a GPT-6 UI의 Pro 보장 (`actions/model.ts`) — 2026-09-05 실측 DOM 기준

2026-09 UI는 **모델 버전 + 생각 강도(power)** 2축이다 (라이브 실측, slot-b):
- 알약(`form button[aria-haspopup]`, `__composer-pill`) 텍스트 `"6\nPro"` — 첫 줄 버전, 둘째 줄 power.
  메뉴가 열려 있는 동안 알약 텍스트는 `"Thinking effort"`로 바뀐다.
- 피커 본문 `[data-testid="composer-intelligence-picker-content"]` 안:
  - `[role=menuitem][aria-label="Select model"]` (텍스트 "6 Pro", `aria-expanded`) → 펼치면/곁에
    `[data-testid="composer-model-picker-slider-advanced-view"]`의 `menuitemradio` `Latest`(=6, checked) /
    `GPT-5.6 Sol` / `GPT-5.5`.
  - `[data-testid="composer-model-picker-slider-simple-view"]`의 `[role=menuitem][aria-label=Power]` 안
    `[role=slider]` (`aria-valuemin=0`, `aria-valuemax=4`; 0=Instant "1 of 5" … 4=Pro "5 of 5") + 상태 문구
    `"Pro, 5 of 5. Use Left and Right arrow keys to adjust power."`. 2026-09-05 관찰에서는 slider가
    숨김·`tabindex=-1`이고 보이는 `Power` menuitem이 `←`/`→` 입력을 받는다. 직접 입력 가능한
    slider도 지원하되 `Home`/`End` 지원은 가정하지 않는다. 값은 즉시 적용되고 메뉴를 닫아도 유지된다.
- 절차: 알약 라벨을 (버전, power)로 파싱. power가 `target`이고 버전 토큰이 없으면(구 UI) 즉시 done.
  새 UI(버전 토큰 있음)는 power가 이미 `Pro`여도 메뉴를 열어 `modelVersion`(Latest) 라디오를 확인한다.
  power가 목표가 아니면 유일한 slider의 정수 bounds/current를 검증(최대 20단계)하고, 보이는
  입력점에 `→`를 한 단계씩 보내 매번 `aria-valuenow` 증가를 확인 → 상태 문구의 첫 토큰이 `Pro`인지 확인 →
  `Latest`가 checked가 아니면 클릭 → `Escape`로 닫기(최대 2회, 5s) → 알약 재확인 `"6\nPro"`.
- 버전 클릭으로 메뉴가 닫히면 다시 열어 목표 라디오의 `aria-checked=true`를 확인한다. 확인 대상
  소실을 성공으로 취급하지 않는다. 알약의 버전 토큰으로 새 UI를 관찰했다면 Power 탐색 실패와
  무관하게 버전 selector를 요구하며, 검증하지 못하면 `model_unavailable/pre_click`으로 끝낸다.
  단, 구 UI의 `Instant / 5.5`도 버전 토큰을 포함하므로 실제 `Pro` 라디오를 선택하고
  `aria-checked`까지 확인한 경우에만 단일 Intelligence 경로임을 인정한다.
- 라벨 정규화는 **모든 줄의 버전 토큰(`^\d+(\.\d+)*$`)을 제거**한 뒤 소문자 비교 — `"6\nPro"`→`pro`,
  `"Instant\n5.5"`→`instant`. 알약 후보가 라벨 집합에 없으면(미지의 중간 power 이름) form 안의
  **텍스트가 있는 유일한 `aria-haspopup` 버튼**을 알약으로 본다(첨부 `+` 버튼은 텍스트가 없다).
- 반환값 = 표시 라벨(`"6 Pro"`). send 결과 `modelLabel`로 supervisor에 전달되어 주간 사용량 원장의 증거가 된다.
- `labels.json`: `{"target":["Pro"],"intelligence":[…],"modelVersion":"Latest"}`. **다른 power/모델로의 fallback 금지.**

### 6.3 Pro 보장 (구 UI, `actions/model.ts`) — 2026-07-27 실측 DOM 기준

현 ChatGPT UI는 model/effort 2단계가 아니라 **단일 "Intelligence" 라디오**다
(라이브 실측: Instant 5.5 / Medium / High / Extra High / Pro + 기타 menuitem).
"Pro Extended"는 곧 이 라디오의 `Pro`다. effort 개념은 코드에서 제거한다.

- **피커 트리거**: composer form 안의 pill 버튼 — `form button[aria-haspopup]` 중
  accessible text가 `labels.json`의 intelligence 라벨 세트(Instant/Medium/High/
  Extra High/Pro …)에 정규화 일치하는 것. 유일하면 그것. (실측: `__composer-pill`
  클래스, 텍스트 "Pro" — 클래스는 식별에 쓰지 않는다.)
- **현재 라벨** = 이 pill의 innerText (readiness의 `modelLabel`도 동일 소스).
- 절차: pill 라벨이 `Pro`면 즉시 done(already_exact; 메뉴 안 연다). 아니면 pill 클릭 →
  `[role="menu"] [role="menuitemradio"]`에서 텍스트 `Pro` 클릭 → 500ms 안정화 →
  `aria-checked="true"` + pill 라벨 재확인.
- `Pro` menuitemradio가 없으면 `model_unavailable`(pre_click).
  **다른 intelligence로의 fallback은 어떤 경우에도 금지.**
- `labels.json`: `{"target": ["Pro"], "intelligence": ["Instant","Medium","High","Extra High","Pro"]}`.
  라벨 정규화: trim + 공백 축약 + casefold + 부가 배지 텍스트(예: "5.5") 제거
  (첫 줄만 사용).

### 6.4 전송 확인

클릭 후 확정 창(`GWP_CONFIRM_WINDOW_MS`, 기본 300s — supervisor가 progress로 생존을
확인하며 기다리므로 넉넉히 잡는다) 내에 다음 셋 모두 관찰되어야 confirmed:
(a) 전송 프롬프트와 일치하는 **새** user 턴(data-message-id가 클릭 전 스냅샷에 없음),
(b) 그 뒤의 새 assistant 턴 (생성 중이어도 됨), (c) non-root `/c/...` URL.
미달 시 `click_uncertain` (post_click) — supervisor가 §5.3으로.

확정 루프 관측 규칙 (2026-07-29 — 대형 프롬프트 스톨의 교훈):
- 루프는 경량 관측(`readTurnsShallow`: id/role만, innerText 없음)으로 돌고, 텍스트는
  매칭 후보 턴만 `readTurnTextById`로 뽑는다. innerText는 강제 layout이라 87KB 턴이
  있는 페이지에서 전량 추출을 반복하면 루프가 분 단위로 늘어진다.
- **앵커는 매칭과 무관하게 즉시 확보**: 새 user 턴이 보이는 순간 `pendingUserTurnId`/
  `pendingConversationUrl`을 잡아 progress로 복제한다 (§5.2).
- (a)의 매칭은 3단 tier다. ① 정확 매칭(`renderedTurnMatchesPrompt`). ② **방금 클릭한
  탭의 유일한 새 user 턴**에 한해 loose 매칭(`renderedTurnMatchesPromptLoose`: 정규화
  프롬프트 ≥4096자, 길이 90%~+200자, 앞뒤 1000자 일치). ③ 같은 조건의 단일 새 턴에
  길이 sanity(`renderedTurnLengthSane`: ≥4096자, 길이 85%~110%)만 보는 single_turn.
  근거: ChatGPT는 user 턴 마크다운을 **렌더링**해 문법 문자가 문서 전체에서 소실된다 —
  2026-07-29 라이브 실측 21,762자 프롬프트가 renderedLen=21336, firstDiff=476,
  tailMatch=0으로 ①②를 모두 구조적으로 실패했다. identity는 텍스트가 아니라 "방금
  클릭한 새 대화 탭 + pre-click baseline + 유일한 새 user 턴"이 이미 보장하므로 ③은
  안전하다. 전 tier 실패 시 길이/first-diff evidence를 progress·에러 detail에 남긴다.

### 6.5 대화 URL·assistant turn id 가변성 (2026-07-27 실측)

ChatGPT는 새 대화 생성 초기에 임시 URL(`/c/WEB:<uuid>`)을 쓰다가 이후 실제 대화 id로
바꾼다. **assistant turn의 data-message-id도 마찬가지로 가변이다**: 전송 확정 시점에는
placeholder id(`request-placeholder-request-WEB:...-0`)였다가 완료 후 실제 UUID로
교체된다(실측). **즉시 안정적인 유일한 anchor는 user turn id다.** 규칙:

- 전송 확정: user turn id가 durable identity. assistant id는 provisional로 기록만 한다.
- poll의 assistant 매칭: 기록된 assistantTurnId가 DOM에 있으면 사용하되, **없으면 실패가
  아니라 user turn 뒤(domIndex)의 첫 assistant turn으로 폴백**한다. poll 결과에 현재
  관찰된 assistant id를 포함하고, supervisor는 placeholder→실 id로 DB를 갱신한다
  (URL 승격과 동일 패턴).
- poll은 현재 페이지에 이 요청의 확정 turn id가 보이면 **URL 문자열이 달라도 절대
  재이동하지 않는다.** 모든 poll 결과에 `currentUrl`을 포함하고, supervisor는 그것이
  기록값과 다른 유효 `/c/` URL이면 DB `conversation_url`을 갱신한다. 단, 한번 non-`WEB:`
  URL로 승격된 포인터는 뒤늦은 `WEB:` 관찰로 되돌리지 않는다.
- 내부 `session.open()`은 진짜 rebind(daemon 재시작 등)에서만 쓴다. 이동 결과가 루트로 리다이렉트되면
  실패로 끝내지 말고 열린 탭들에서 promptSha 기반 reconcile을 먼저 시도한다.

## 7. 생성 완료 판정 (poll)

complete 판정 조건 (모두 충족):
1. stop 버튼 부재,
2. 마지막 assistant 턴의 answerText sha256이 3초 간격 2회 연속 동일,
3. 답변 액션 바(복사 버튼 등, `[data-testid*="copy"]` 계열) 노출.

주의: 스트림 종료 직후 텍스트 공백 갭이 존재한다 (v1 poll 버그의 교훈,
`lib/commands/poll.mjs`) — stop 버튼이 사라져도 answerText가 비어 있으면 complete가 아니다.
답변 추출은 마지막 visible assistant 턴을 기준으로 하되 저장 assistant id가 사라졌으면 그 턴으로
폴백한다. 파일 엔티티/inline download subtree와 그 wrapper의 action-only `Download` 텍스트는
answer markdown/hash에서 제외한다.
정제 후 본문이 비어도 artifact control이 하나 이상이면 유효한 artifact-only complete다.

complete 안정화 시 artifact control이 0개이고 정제 전 답변에 파일명 또는 명시적인
download/file/attachment(한국어 포함) 힌트가 있거나 정제 본문이 비었을 때만, poll deadline 안에서 최대 8초 동안
500ms 간격으로 지연 렌더를 재관찰한다. 힌트가 없는 일반 답변은 유예 없이 즉시 complete한다.
poll deadline이 먼저 오면 빈 artifact로 종결하지 않고 generating을 반환해 다음 poll에서 잇는다.

### 이미지 세트의 완료와 다운로드 (2026-09-05 실측)

이미지 배치는 새 대화 하나당 사용자 턴 하나를 계약으로 한다. 확정 userTurnId와 유일한
사용자 턴이 다르거나 후속 사용자 턴이 추가되면 수집을 거부한다. 이 경계 안에서 이미지가
텍스트 assistant 메시지 노드의 형제로 렌더되는 경우도 수집한다.

현재 이미지 세트 UI는 큰 `div[role=button]` 미리보기와 `button` 썸네일 N개다.
각 썸네일에 `img[alt="Generated image"]`가 세 겹으로 들어 있으므로 img 수를 세지 않는다.
단일 이미지의 `Generated image: 제목` 라벨도 같은 생성 이미지로 식별한다.
단일 이미지 뷰어의 dialog 이름도 해당 제목이므로 공통 `Image tools` 그룹으로 식별한다.
이미지 전송 확정은 문단·줄바꿈 공백을 정규화한 전체 프롬프트도 비교한다. 짧은 이미지
입력의 공백 렌더 차이로 불필요하게 확정 창이 끝날 때까지 기다리는 일을 막는다.
사용자 턴 뒤의 접기·펼치기 버튼은 해당 DOM 컨트롤의 실제 텍스트가 접미사로 일치할 때만
프롬프트 비교에서 제외한다. 일반 텍스트 위임의 판독 규칙에는 적용하지 않는다.
썸네일이 있으면 썸네일 버튼, 없으면 개별 미리보기를 센다. stop 부재, 응답 복사 액션,
실제 이미지 로드, 3초 동안 안정된 개수를 확인한다. 본문 텍스트가 없어도 이미지 응답은 완료된다.
복사 액션은 영어·한국어 공통 selector를 쓰되 마지막 이미지 뒤로 범위를 좁혀 사용자 메시지의
복사 버튼을 완료 증거로 사용하지 않는다. 이미지 없는 텍스트 응답은 assistant anchor를 쓴다.
대화 재개 직후 `Generated image` 미리보기만 있고 이미지가 로드되지 않은 상태는 계속
generating으로 남긴다. Copy response가 보여도 빈 갤러리를 0장 완료로 취급하지 않는다.
갤러리 이름조차 없는 빈 assistant도 같은 원칙이다. 이미지 없는 텍스트 응답은 원문을 보존해
미완료 사유를 확인할 수 있게 한다. 전체 컨트롤 수가 예상 수와 같기 전에는 입력 ID를
이미지 순서에 결속하거나 다운로드하지 않는다. 일부 썸네일 누락으로 순서가 밀리는 것을 막는다.

다운로드는 요청의 user anchor를 다시 확인하고 썸네일을 순서대로 선택한다. 큰 미리보기가
선택한 썸네일의 이미지로 바뀐 뒤 원본 뷰어를 연다. `Save` 클릭 전에 다운로드 이벤트를
등록한다. 단일 이미지는 직접 다운로드되며, 다중 세트는 `Save` 메뉴의 `Download image`를
누른다. `Download N images in this series`와 정확히 구분한다.
일반 파일 패널의 `Download`와 구분하며 기존 download 이벤트 경로로 파일을 저장하고
뷰어를 닫는다. URL은 비교에만 쓰고 구성·fetch하지 않는다. 파일명은 입력 ID와 실제 디코딩한
확장자를 사용하므로 공급자의 동일 파일명이 서로 덮어쓰지 않는다. 의미·순서·문자 품질은
실제 산출물을 보는 검수가 필요하다. 일반 텍스트 다운로드는 지정 assistant anchor를 벗어나지 않는다.

**artifact 컨트롤 발견 (2026-07-28 실 DOM 기준)**: 현 ChatGPT는 인라인 `a[download]`가
아니라 **답변 본문의 "파일 엔티티" 버튼**으로 파일을 노출한다 (실측: `<button
aria-label="numbers.txt" class="behavior-btn …">numbers.txt</button>`, 파일 아이콘 svg 포함).
발견 규칙:
- assistant 턴 안에서 파일명(§6.1 FILENAME 정규식)을 accessible name/텍스트로 가진 버튼을
  나열. 각 컨트롤의 label=파일명, index=등장 순서.
- 다운로드 실행: 그 버튼 클릭 → 열리는 미리보기 패널에서 `aria-label="Download"` 버튼을
  Playwright download 이벤트를 arm한 채 클릭 → 저장. (실측: 파일 버튼 클릭 시 패널에
  `Download` 버튼 등장.) 인라인 `a[download]`도 있으면 그대로 지원(폴백).

다운로드: Playwright download 이벤트로 outbox에 저장 → supervisor가
`requests/<id>/artifacts/`로 이동 + sha256/size 기록. `.tar.gz` 같은 복합 확장자 보존.
CDP 이벤트와의 이중 상관은 하지 않는다.

managed 슬롯의 daemon은 컨테이너 경로 `/outbox/<file>`을 RPC `outboxPath`로 반환한다.
supervisor는 `/outbox/` 경계 안의 경로만 슬롯의 host bind mount
`<state>/slots/<slotId>/outbox/<file>`로 매핑하고, host 경계를 다시 검사한 뒤 그 host
경로에서 sha256/size를 검증하고 요청 artifact로 이동한다. unmanaged 테스트 슬롯은 daemon이
반환한 host 경로를 그대로 사용한다. 유효하게 매핑된 managed outbox 파일은 저장 성공과
검증/저장 실패 모두 정리해 `.gwp-*` 임시 파일이나 최종명이 슬롯 outbox에 남지 않게 한다.

**일반 파일의 artifact 실패 정책**: 컨트롤당 최대 2회 시도. 그래도 실패하면 — 성공한 artifact는 전부
보존(행+파일), 요청은 **complete로 종결**하되 envelope `message`에 실패 컨트롤
수/라벨을 명기한다(`errorKind`는 null 유지). 수 시간짜리 Pro 답변을 다운로드 플레이크에
인질 잡지 않는다.

## 8. 슬롯 / 컨테이너 런타임

`config/slots.json`:

```json
{ "image": "home-server/gpt-webai-pro-slot:latest",
  "maxConcurrent": 3,
  "slots": [
    {"id":"slot-a","account":"a","port":19301},
    {"id":"slot-b","account":"b","port":19302},
    {"id":"slot-c","account":"c","port":19303} ] }
```

### 8.1 슬롯 = 계정 (2026-07-27 재편)

**슬롯 하나가 계정 하나이고 프로필 하나이고 로그인 하나다.** 같은 계정에 프로필 여러 개를
두지 않는다 — 로그인 관리 표면을 계정 수로 고정하기 위해서다. 슬롯당 동시 요청은
`maxConcurrent`(기본 3)까지 허용하며, 동시성은 §6.2의 탭 멀티플렉싱이 담당한다.
컨테이너 수명은 비종결 행 수가 아니라 live CLI owner로 결정한다. 비종결 요청이 있어도
owner가 없으면 정지하며, 다음 resume 때 같은 profile로 재기동한다.

- **할당**: idle & cooldown 지난 슬롯 중, `last_used_at`이 가장 오래된 **계정**의 가장 오래된
  슬롯 (계정 간 공평 로테이션). v1의 cohort cursor 영속 상태는 두지 않는다 — LRU가 같은 효과.
- **할당 가능 조건**: `state='idle'` 또는 (`state='provider_limit'` 그리고 `cooldown_until<=now`).
  `needs_login`/`busy`는 할당 불가. `provider_limit` 슬롯은 run 흐름의 readiness 확인이
  자연 재검증이 되고, 성공적으로 사용되면 idle로 복귀한다.
- **needs_login 복귀**: 운영자가 로그인 시딩 후 `cleanup --apply` 실행 →
  needs_login 슬롯들을 daemon `readiness()`로 재검사해 ready면 idle로 전환.
- **테스트 심**: 슬롯 항목에 `"unmanaged": true`가 있으면 supervisor는 docker를 일절
  호출하지 않고 해당 `port`(127.0.0.1)에 바로 접속한다. supervisor e2e는 이 모드로
  mock daemon에 붙는다. 프로덕션 slots.json에는 사용하지 않는다.
- **provider_limit**: 슬롯 `cooldown_until = now + 3분` 기록 후 다른 계정 슬롯으로.
- **컨테이너**: 이름 `gwp-<slotId>`. supervisor가 `docker` CLI로 직접 관리 (compose 없음).
  create 인자(코드 한 곳에 정의): `--memory 3g --cpus 2 --pids-limit 1024 --shm-size 1g
  --security-opt no-new-privileges --cap-drop ALL --user <uid>:<gid> --restart no`
  마운트: `profile→/profile(rw)`, `inbox→/inbox(ro)`, `outbox→/outbox(rw)`.
  이름만으로 소유권을 인정하지 않는다. 세 destination 각각 유일한 bind mount여야 하고 source의
  canonical path가 현재 state의 슬롯 경로와 같아야 한다. 불일치·누락·확인 불가는 fail-closed이며
  토큰 교체 전 거부한다. start/stop/remove는 재검증한 Docker ID로 실행해 이름 재사용 경합에서도
  다른 컨테이너를 조작하지 않는다. 병렬 작업본은 state root, 슬롯 ID, daemon 포트를 모두 분리한다.
  publish는 `-p 127.0.0.1:<port>:<port>` (daemon RPC) 하나뿐. CDP는 컨테이너 내부
  127.0.0.1 전용으로 publish하지 않는다.
- **entrypoint**: Xvfb :99 → Chromium(CDP 9222, `--user-data-dir=/profile`,
  chatgpt.com 오픈) → CDP ready 대기 → daemon 기동(`0.0.0.0:<port>` listen —
  컨테이너 밖에서는 host 127.0.0.1로만 publish되므로 안전).
  ready 판정은 supervisor가 TCP 연결 + `health` RPC로 한다.
- **정지**: run/resume 종료 시 해당 슬롯의 다른 live `owner.lock`이나 유지보수 owner가
  없으면 `docker stop`. active DB 행은 장기 작업의 durable state이지 runtime lease가 아니다.
- **cleanup**: (a) live owner가 없는 managed runtime 정지(요청/profile/쿠키는 보존),
  (b) `needs_login` 슬롯 readiness 재검사. `--dry-run`이 기본, `--apply`로 실행.
- **환경 오버라이드**: `GWP_BASE_URL`(기본 `https://chatgpt.com`, 테스트 하네스용),
  `GPT_WEBAI_PRO_STATE_DIR`, `GPTPRO_TIMEOUT`(기본 10800s).

## 9. 공개 CLI / envelope 계약

기존 `gptpro` 호출 계약을 그대로 승계한다 (전역 CLAUDE.md의 envelope 불변식 호환).

커맨드: `run | resume | status | cleanup | release | smoke | login | keepalive | reap`

### 9.1 `login --slot <id>` — 원클릭 로그인 시딩

로그인(자격증명·2FA·CAPTCHA)은 자동화하지 않는다 — 사람이 직접 하되, 컨테이너 안
Chrome에 손이 닿게 만든다:

1. 해당 슬롯에 비종결 요청이 없는지 확인(있으면 거부).
2. 컨테이너를 **login 모드**로 재생성: env `GWP_LOGIN_MODE=1` → entrypoint가 Xvfb를
   1440x900으로 띄우고 x11vnc(-localhost)+noVNC(websockify)를 추가 기동.
   noVNC 포트 = `port + 600` (19901..), `-p 127.0.0.1:<novnc>:<novnc>`만 추가 publish.
   (인증: loopback 전용 + 단일 사용자 호스트 — daemon 포트와 동일한 신뢰 모델. VNC
   비밀번호는 두지 않는다.)
3. `사용자에게 http://127.0.0.1:<novnc>/vnc.html 안내`와 대기 경과는 stderr에 출력하고,
   5초 간격으로 daemon `readiness`를 폴링(최대 15분). stdout은 마지막 JSON 한 객체+LF만.
4. `ready` 관찰 → 컨테이너 정지 → 슬롯 idle 기록 →
   `{"ok":true,"slot":"slot-a","state":"ready","novncUrl":"http://127.0.0.1:19901/vnc.html"}`
   (exit 0). 타임아웃도 컨테이너 정지 후 `state:"needs_login"`,
   `errorKind:"login_timeout"` JSON(exit 0). SIGINT/SIGTERM은 정지 후
   `errorKind:"login_aborted"` JSON(exit 130). 컨테이너/RPC 오류는
   `errorKind:"daemon_unreachable"` JSON(exit 0). 슬롯 없음/비종결 요청은 exit 2.
- 이미지에 `x11vnc novnc websockify` 패키지 추가. 일반 모드에서는 어느 것도 기동하지
  않고 포트도 publish하지 않는다.

### 9.2 `keepalive` — 세션 사용-갱신 + 만료 조기 감지

세션은 쓰지 않으면 죽는다. `keepalive`는 모든 구성 슬롯에 대해(상태 무관, 단 비종결
요청이 있는 슬롯은 스킵 — 이미 살아 있음): 컨테이너 기동 → `readiness`(페이지 로드가
곧 토큰 갱신) → 원래 꺼져 있었으면 정지. `ready→idle`, `needs_login`, `provider_limit`의
확정 관찰만 slots.state에 기록하며 `unknown`/기동·RPC 오류는 기존 DB 상태를 보존한다.
출력은 `{"ok":true,"slots":[{"id","state","probe"}]}`. `state`는 DB 최종 상태,
`probe`는 `ready|needs_login|provider_limit|unknown|unreachable`; active-request 스킵은
`unknown`. exit는 항상 0(needs_login/unreachable도 실패가 아니라 관찰 결과다).

repo에 systemd user unit 2개를 둔다 (`systemd/gpt-webai-pro-keepalive.service|.timer`,
`OnCalendar=*-*-* 09:20`, `RandomizedDelaySec=600`, README에 설치법). 설치는 운영자 몫.

```
gpt-webai-pro run [--prompt-file P | 프롬프트 인자 | stdin] [--file F]... [--timeout-seconds N]
gpt-webai-pro resume --session req_... [--timeout-seconds N]
gpt-webai-pro status [--json]   # 슬롯별 state/cooldown + 비종결 요청(id,status,slot,경과시간) 목록
gpt-webai-pro cleanup [--dry-run|--apply]
gpt-webai-pro release --session req_...   # 강제 종결(failed 처리) + 컨테이너 정지
gpt-webai-pro smoke                       # 라이브 1회 왕복. GWP_LIVE=1 필수
```

§9의 요청 envelope는 **run / resume / release**에 적용된다.
`status`/`cleanup`/`smoke`/`login`/`keepalive`는 자체 JSON을 출력한다 (status:
`{"ok":true,"slots":[{"id","account","state","cooldownUntil","lastUsedAt","activeRequests"}],`
`"requests":[{"id","status","slotId","ageSeconds","conversationUrl"}]}` — 비종결 요청만;
cleanup: `{"ok":true,"dryRun":bool,"actions":[{"kind","target","detail"}]}`).

run/resume/release의 stdout은 항상 **정확히 하나의 JSON 객체 + LF**:

```json
{ "ok": true, "hardFailure": false, "networkDisconnected": false, "usageError": false,
  "status": "complete|running|recovering|needs_user_action|failed",
  "sessionId": "req_...", "resumeCommand": "gpt-webai-pro resume --session req_...",
  "nextCommand": null,
  "answer": "...", "answerPath": "...", "answerSha256": "...",
  "artifacts": [{"filename":"...","path":"...","sha256":"...","sizeBytes":0}],
  "errorKind": null, "message": null }
```

규칙 (전역 계약과 동일):
- `hardFailure:true` + `networkDisconnected:true`는 **직접 증거로 네트워크 단절이 입증될 때만**
  (예: chatgpt.com 네비게이션이 DNS/오프라인으로 실패). exit 1은 이 경우뿐.
- 빈 프롬프트: exit 0, `usageError:true`, `status:"needs_user_action"`. 브라우저 접촉 없음.
- timeout: exit 0, `status:"running"`, 동일 sessionId의 resumeCommand. 새 요청 생성 금지.
- 그 외 인풋 오류 exit 2, 내부 오류도 envelope로 감싸 exit 0 (`status:"failed"`) —
  단 envelope 출력 자체가 불가능한 파국만 exit 70.

## 10. 에러 분류 (전체 폐쇄 목록)

| errorKind | status | 의미 |
| --- | --- | --- |
| `needs_login` | needs_user_action | 슬롯 로그인 필요 (어느 슬롯인지 message에) |
| `provider_limit` | recovering | 전 계정 rate-limit, cooldown 후 resume |
| `model_unavailable` | needs_user_action | 피커에 Pro/Extended 라벨 부재 |
| `send_uncertain` | needs_user_action | reconcile로도 증명 불가 (§5.3) |
| `pool_busy` | recovering | idle 슬롯 없음 |
| `daemon_unreachable` | failed | 컨테이너/daemon 기동 실패 |
| `network_disconnected` | (hardFailure) | 직접 증거 있는 네트워크 단절 |
| `internal` | failed | 그 외 |

이보다 세분화된 kind를 추가하지 않는다. 세부는 `message`/`error_detail`/log.jsonl로.

## 11. 테스트 전략 (3겹)

### 11.1 unit (`test/unit/`)
db 스키마/쿼리, 멱등성 상태기계(§5 전이표 전체 — 특히 armed에서 죽은 뒤 resume 경로),
envelope 직렬화(전역 계약 케이스: 빈 프롬프트, timeout, hard failure), 슬롯 LRU/쿨다운.

### 11.2 daemon e2e — fake-chatgpt 하네스 (`test/fake-chatgpt/`)
node http 서버 + 단일 SPA. **selectors.ts가 쓰는 DOM 계약만** 모사한다
(data-testid/role/aria/data-message-id 구조). 쿼리 파라미터로 시나리오 선택:

`happy`(스트리밍 답변), `login-wall`, `rate-limit`, `model-missing`,
`post-stream-gap`(stop 사라진 뒤 1.5s 후 텍스트 등장), `attachments`(칩 + 중복 파일명 리네임),
`artifacts`(다운로드 2개, `.tar.gz` 포함), `slow`(waitMs 초과용).

실제 chromium(playwright-core, headless)을 `GWP_BASE_URL=http://127.0.0.1:<port>`로 하네스에
붙여 daemon RPC 전체(send/reconcile/poll/download/readiness)를 검증한다.
chromium 해석 순서: `$CHROME_BINARY_PATH` → `~/.cache/ms-playwright/chromium-*` →
`npx playwright install chromium` 안내 후 실패.

### 11.3 supervisor e2e (`test/supervisor.e2e.test.ts`)
in-process mock daemon(WS 서버)으로 supervisor 오케스트레이션 검증. 필수 시나리오:
- send 중 소켓 단절 → uncertain → resume → reconcile found → 재클릭 0회로 complete
- reconcile not-found → attempt 2 재전송 → complete. attempt 2도 실패 → needs_user_action
- 동시 run 2개 → 서로 다른 슬롯 → 계정 로테이션 확인
- provider_limit 슬롯 스킵 + 쿨다운
- timeout → running envelope → resume → complete

### 11.4 게이트
`npm run build`(tsc strict 통과가 곧 린트) + `npm test`(위 전부). 그 외 게이트 없음.
`scripts/container-smoke.sh`(이미지 빌드 + `--network host` 컨테이너로 하네스 왕복)는
릴리스 전 수동 1회. 라이브 스모크(`smoke`)는 컷오버 단계에서 별도 수행.

## 12. v1으로부터의 컷오버 (이 구현의 범위 밖, 참고용)

1. 구현 + 게이트 통과 + container-smoke 후, 운영자가 old 스택 정지.
2. `<v1 state>/slots/slot-NN/state/browser-profile` → `<v2 state>/slots/slot-NN/profile` 복사
   (로그인 세션 승계, 재로그인 불필요).
3. `~/.local/bin/gptpro`를 v2 CLI exec으로 교체. **`gptxhigh`는 폐기(삭제)**.
4. `gpt-webai-pro smoke`로 라이브 1회 검증 → 이후 실사용.
5. v1 스택/PR72 처분은 사용자 결정 사항.

## 13. 구현 시 참조할 v1 소스 (읽기 전용)

라이브 검증이 끝난 로직이므로 새로 발명하지 말고 이식한다. 경로는 PR72 워크트리
`~/Documents/Programming/home-server-infra-gpt-webai-slot-pool-pr/stacks/gpt-webai-slot-pool/`:

- `provider/chatgpt-playwright/lib/commands/upload-only.mjs` — 칩 시맨틱 탐지 (최종본)
- `provider/chatgpt-playwright/lib/turns.mjs` — 턴/생성중/답변 추출
- `provider/chatgpt-playwright/lib/send-confirmation.mjs` — 새 턴 증명 + prompt-sha reconcile
- `provider/chatgpt-playwright/lib/commands/poll.mjs` — post-stream 갭 처리
- `provider/chatgpt-playwright/lib/browser-composer.mjs` — 컴포저 셀렉터
- `contracts/ui-labels-r14/model-effort-labels.tsv` — Pro 라벨 세트
- `Dockerfile` + `scripts/slot-entrypoint.sh` — chromium/xvfb 기동 패턴

**이식 금지 대상**: root-selector/바인딩 해시, contracts/r13.mjs, scroll-proof, artifacts.mjs,
session-rebind의 hydration 상태기계(→ 내부 `session.open()`+`poll`로 충분), R12 일체.
