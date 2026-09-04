# Runbook — gpt-webai-pro (ChatGPT Pro 웹 위임, 부활본)

> **2026-08-25 사용자 지시로 부활.** Mac Orca 화면 자동화(chatgpt-pro-ask)가 XPC 파일
> 다이얼로그 포커스·IME·창 유실로 반복 실패해, 홈서버의 슬롯 데몬 방식을 되살렸다.
> 이 경로는 헤드리스 Chromium 슬롯 3개(계정 a/b/c) + SQLite로 중복 전송 방지·resume·
> envelope 계약을 데몬이 보장한다. **화면 자동화보다 이 경로를 우선한다.**
>
> ## Mac에서 호출 (데몬은 홈서버 Linux에서 돈다)
>
> ```bash
> # 상태
> ssh home 'export PATH="$HOME/.local/bin:$PATH"; gpt-webai-pro status'
> # 전송: 번들을 홈서버로 복사 후 run (프롬프트는 stdin, 첨부는 --file)
> scp bundle.zip prompt.md home:rf_work/
> ssh home 'export PATH="$HOME/.local/bin:$PATH"; cd ~/rf_work; \
>   setsid nohup bash -c "cat prompt.md | gpt-webai-pro run --file ~/rf_work/bundle.zip \
>   > envelope.json 2>run.log" < /dev/null >/dev/null 2>&1 &'
> # 회수: envelope.json 의 status·answerPath 확인, running 이면 resumeCommand 로 이어감
> ssh home 'cat ~/rf_work/envelope.json'
> ssh home 'export PATH="$HOME/.local/bin:$PATH"; gpt-webai-pro resume --session req_...'
> ```
>
> - slot-a/b/c 중 idle 슬롯이 있으면 로그인 없이 전송된다. `needs_login`이면 그 슬롯만
>   `gpt-webai-pro login --slot <id>` 후 noVNC(`http://<home>:1930N+600/vnc.html`)로 사람이 로그인.
> - **회수를 즉시 하려면 느린 status 폴링을 쓰지 마라.** 데몬은 답이 3초 안정되면(STABLE_GAP_MS)
>   run/resume가 즉시 반환하고 run.log에 `EXIT=`·env.json에 status를 쓴다. 그러므로:
>   (1) `gpt-webai-pro run/resume`를 실행하는 ssh 명령을 **Bash 도구 run_in_background로 띄우면**
>       프로세스가 끝나는 즉시(=완료 ~3초 후) 하네스가 알려준다 — 폴링 불필요. 이게 표준이다.
>   (2) 굳이 detach했으면 `until grep -q "EXIT=" run.log; do sleep 2; done`처럼 **2초 간격**으로
>       완료 마커를 감시한다. status를 수십 초 간격으로 확인하면 그 지연만큼 늦게 잡힌다(데몬 문제 아님).
> - **send_uncertain은 실패가 아니다.** 대용량 첨부에서는 user 턴 텍스트 매칭이 확정 창 안에
>   안 끝나 첫 envelope가 `needs_user_action`/`send_uncertain`으로 나올 수 있다. 재전송하지 말고
>   같은 세션을 `resume`하면 turn_anchor로 reconcile돼 generating→complete로 회수된다.
>   (근본 완화는 배포됨: 확정은 user 턴+대화 URL로 판정. 그래도 매칭 실패 시 이 resume 경로가 안전망.)
> - answer는 홈서버 state(`~/.local/state/gpt-webai-pro`)와 envelope의 `answerPath`에 저장된다.
>   Mac으로 회수: `ssh home cat <answerPath>` 또는 scp.
> - 화면 자동화 경로(chatgpt-pro-ask)는 이 경로가 막힐 때의 보조로만 남긴다.

---

# Runbook — gpt-webai-pro (ChatGPT Pro 웹 위임 v2)

v1(`gpt-webai-lifecycle`/slot-pool, `gptxhigh`)을 대체한 2세대. 구현은 `stacks/gpt-webai-pro`
(TypeScript 슬롯 데몬 + SQLite). 상세 운영은 그 스택의 `README.md`, 설계는 `DESIGN.md`.

## 진입점 (이것만 쓴다)

```bash
gptpro "프롬프트"                         # = gpt-webai-pro run
printf '%s\n' "프롬프트" | gptpro
gptpro --file PATH "프롬프트"             # 첨부 반복 가능, 폴더는 zip/tar
gpt-webai-pro resume  --session req_...   # running/uncertain/pool_busy 이어가기
gpt-webai-pro status                      # 슬롯 상태 + 비종결 요청
gpt-webai-pro release --session req_...   # 강제 종결(사용자 지시 시만)
gpt-webai-pro cleanup [--dry-run|--apply] # 고아 컨테이너/needs_login 재검사
gpt-webai-pro login   --slot slot-a|slot-b|slot-c   # noVNC 원클릭 로그인 시딩
gpt-webai-pro keepalive                   # 세션 유지 + 만료 조기 감지 (systemd 타이머 09:20)
```

`gptxhigh`, `gpt-webai-lifecycle`, `resume --kind`, `show`, `--slot slot-NN`, cohort,
`/broker-attachments`, auth-seed 는 전부 폐기됐다.

## Envelope 계약

run/resume/release stdout = JSON 한 객체.
`{ok, hardFailure, networkDisconnected, usageError, status, sessionId, resumeCommand,
answer, answerPath, answerSha256, artifacts, errorKind, message}`.

- `hardFailure:true`+`networkDisconnected:true`(exit 1) = 직접 증거로 chatgpt.com 네트워크 단절 입증 시만.
- 그 외 전부 exit 0 envelope. `status ∈ complete|running|recovering|needs_user_action|failed`.
- 빈 프롬프트 = exit 0 `usageError:true`, `status:"needs_user_action"`, 브라우저 무접촉.
- errorKind 폐쇄 목록: `needs_login`(needs_user_action) / `provider_limit`(recovering) /
  `model_unavailable`(needs_user_action) / `send_uncertain`(needs_user_action) /
  `pool_busy`(recovering) / `daemon_unreachable`(failed) / `network_disconnected`(hardFailure) / `internal`(failed).

## 불변식 (안전)

- **중복 전송 절대 금지.** timeout·불확실은 새 전송이 아니라 같은 `sessionId`의 `resumeCommand`로만 이어간다.
  중복 방지는 flock(요청별 send.lock) + send_attempts 행 + ChatGPT user-turn id 앵커가 담당한다.
- `status:"uncertain"`/`send_uncertain`은 resume이 reconcile로 판정(착지 턴 있으면 회수, 없으면 fail-closed).
  사람이 임의로 재전송하지 않는다.
- Pro는 수 시간 걸리는 게 정상. `GPTPRO_TIMEOUT`(기본 10800s) 후 `status:"running"`은 실패가 아니다.
- 상태 저장소 `$HOME/.local/state/gpt-webai-pro`(SQLite+evidence+프로필)와 `gwp-slot-*` 컨테이너를
  직접 삭제/정지/`docker exec`하지 않는다. 정리·복구는 `cleanup`/`release`만.

## 자주 나오는 상황

- **needs_login**: 슬롯 세션 만료. `gpt-webai-pro login --slot <id>` → stderr에 안내되는
  `http://127.0.0.1:<port+600>/vnc.html`을 로컬 브라우저로 열어 사람이 직접 로그인 → 자동 저장·정리.
  (계정 3개 = slot-a/b/c, 포트 19301-3.)
- **pool_busy / recovering**: 모든 슬롯이 maxConcurrent(3)까지 참. `resumeCommand`로 재시도.
- **running (timeout)**: Pro 장시간 생성 중. 같은 `resumeCommand`로 회수.
- **터미널/SSH 중단 후**: `sessionId`가 있으면 `resume`으로 회수. 방치된 요청은 `cleanup --apply`가
  자가 치유(비종결 요청 없는 busy 슬롯 idle 복구, 고아 컨테이너 정지).

## 쿠키/로그인 영속 (중요)

컨테이너 정지는 반드시 supervisor(`stop`/`release`)를 통한다. daemon이 CDP `Browser.close`로
Chromium을 클린 종료해야 세션 쿠키가 flush된다 — 하드킬은 로그인/회전 쿠키를 유실시킨다.
직접 `docker kill`/`docker stop`(짧은 유예)을 쓰지 않는다.

## Chrome/CDP 복구

v2는 슬롯 컨테이너 내부에서 chromium+Xvfb+daemon을 스스로 기동하므로 호스트 Chrome 설치가 필요 없다.
`daemon_unreachable`이 반복되면 이미지 재빌드(`stacks/gpt-webai-pro/container/Dockerfile`) 또는
`docker logs gwp-slot-<id>`로 진단한다. 슬롯 컨테이너를 수동으로 만지지 말고 supervisor 경로로 복구한다.

## 심층 리서치 모드 (`--deep-research`, 2026-08-25 추가)

ChatGPT "심층 리서치(Deep research)"로 프롬프트를 돌린다. `gpt-webai-pro run --deep-research --file <zip> ...`.

- **동작 순서(중요)**: 컴포저 `+` 메뉴에서 "Deep research"를 고르는데, **프롬프트 입력(fill) 뒤에** 선택한다.
  `fill()`이 먼저 실행되면 이미 켠 심층 리서치 pill이 지워진다(2026-08-25 실측). send.ts 순서:
  `compose(fill) → ensure_tool(deep 선택) → attach → send → Start 클릭`. 첨부는 pill을 지우지 않는다.
- **되묻는 절차**: 전송 후 "리서치 계획 카드"가 뜨고 **Start(25초 카운트다운)** 버튼이 있다. 데몬이
  `confirmDeepResearchStart`로 Start를 눌러 즉시 시작시킨다(없으면 자동 시작/자유형 질문에 맡김).
  자유형 명확화 질문이 오면 같은 대화에 resume/follow-up으로 답해야 리서치가 시작된다.
- **send_uncertain은 정상**: 대용량 첨부에선 확정 창이 실패해 `send_uncertain`이 나오지만 전송은
  실제로 landing하고 리서치는 시작된다. `resume --session <id>`로 reconcile + 리포트 폴링(10~30분).
- **활성 감지**: 메뉴가 닫힌 뒤 컴포저(form) 안의 "Deep research" pill로만 판정한다(열린 메뉴 텍스트 오탐 주의).

## 배포 함정 — 상주 슬롯 컨테이너는 옛 이미지를 재사용 (2026-08-25 실측)

데몬 소스를 고쳐 이미지를 재빌드해도 **`gwp-slot-<x>` 컨테이너가 running이면 그대로 재사용**된다.
`Docker.ensure()`는 `!state.running`일 때만 rm+create+start하고, running이면 이미지 변경을 확인하지 않는다.

- 새 이미지 반영 절차: `docker build -t home-server/gpt-webai-pro-slot:latest .` 후
  **`docker stop -t 40 gwp-slot-a && docker rm gwp-slot-a`** (다음 run이 새 이미지로 재생성).
- `-t 40` graceful stop은 entrypoint의 SIGTERM 트랩이 Chromium 쿠키를 flush하므로 **쿠키 유실 없음**
  (hard `docker kill`은 금지 — 쿠키 유실). 프로필은 호스트 바인드 마운트라 이미지 교체와 무관하다.
- CLI/supervisor(호스트 `dist`)는 즉시 반영되지만 **데몬 send 로직(컨테이너 `/app/dist`)은 컨테이너
  재생성 전까지 반영 안 된다**. 순서 관련 버그를 디버깅할 때 반드시 컨테이너 재생성 여부를 먼저 확인.

---

# 심층 리서치 "배치" 파이프라인 (2026-08-26 실전 확립)

여러 건의 심층 리서치를 무인 직렬로 돌려 **각 리포트를 완전한 markdown 파일로 회수**하는 전체 절차.
데몬(gpt-webai-pro)이 아니라 **전용 Playwright 하네스**로 브라우저를 직접 몬다. 데몬은 심층 리서치
생명주기(계획카드→Start→장시간 아티팩트 리포트)와 맞지 않고 send_uncertain→resume이 리서치를 깨뜨린다.

**스크립트 영속 위치**: `~/.codex/runbooks/gpt-webai-pro-deepresearch/`
(`batch.cjs` 직렬 드라이버, `diag-entry.sh` 컨테이너 기동, `retrieve.cjs`/`step.cjs`/`capture.cjs` 보조).
Mac·홈서버 양쪽에 있다. 컨테이너 안에서 `node /work/…`로 실행한다.

## 아키텍처

- **slot-a 프로필 사본** 위에서 도는 별도 컨테이너를 쓴다(실 slot-a·데몬을 안 건드림).
  `cp -a ~/.local/state/gpt-webai-pro/slots/slot-a/profile ~/rf_work/diag/profile`.
- 컨테이너: `home-server/gpt-webai-pro-slot:latest` 이미지를 `--entrypoint bash /work/diag-entry.sh`로 띄운다.
  diag-entry가 Xvfb + Chromium(`--user-data-dir=/profile --remote-debugging-port=9222`)을 올리고 sleep.
  마운트: profile→/profile, outbox(회수 파일)→/outbox, 스크립트+zip+프롬프트 디렉토리→/work(ro).
- 하네스는 `chromium.connectOverCDP('http://127.0.0.1:9222')`로 붙어 페이지를 조작한다.

## 한 도메인 생명주기 (batch.cjs `fire`)

1. **Chat 모드 강제**: 상단 `Chat` 탭 클릭 → `+`메뉴에 "Deep research"가 보일 때까지 최대 3회.
   안 보이면 **Work 모드**(5.6 Sol, 심층 리서치 없음)이거나 레이트리밋이므로 그 도메인 스킵/백오프.
2. 프롬프트 입력(`fill`).
3. **심층 리서치는 입력 뒤에 선택**(`+`→"Deep research" 잎 클릭). `fill`이 먼저면 pill이 지워진다.
   form 안 pill로 활성 확인.
4. zip 첨부(`input[type=file]`). 첨부는 pill을 지우지 않는다.
5. 전송. 직후 뜨는 **리서치 계획 카드의 "Start"**(25초 카운트다운) 클릭.
6. **fire는 페이지를 닫지 않는다**(중요). fresh 네비게이션엔 리포트가 렌더되지 않으므로,
   같은 in-session 페이지에서 회수해야 한다.

## 리포트 회수 (batch.cjs `tryRetrieve` — 가장 어려웠던 부분)

심층 리서치 리포트는 **closed shadow DOM 아티팩트**라 `innerText`·`getByText`·표준 copy·페이지 내
클립보드가 전부 안 통한다(헤드리스 클립보드는 stale). 유일하게 되는 경로:

1. `page.context().newCDPSession(page)` → **`Browser.setDownloadBehavior {behavior:'allow', downloadPath:'/outbox'}`**.
2. scroll-to-bottom 후 리포트 카드의 **expand(↗) 버튼을 좌표 그리드로 스캔 클릭**해 캔버스를 연다.
   좌표는 도메인마다 ±15px 변동 → `x∈[835,850,828,842,820,860] × y∈[605,565,525,645,485,585]`.
3. 캔버스 우상단 **다운로드(982,24) → "Export to Markdown"(884,102)** 클릭.
   → downloadPath에 `deep-research-report.md`(완전한 전문)가 저장됨 → `<label>-final.md`로 rename.
4. 각 조합 후 파일 생성·크기(>2KB) 확인, 성공 시 break. (Copy contents 클립보드·`waitForEvent('download')`는
   헤드리스에서 불안정 → 반드시 setDownloadBehavior 경로.)

## 필수 불변식·함정 (전부 실전에서 터진 것)

- **fire 페이지 유지**: 닫으면 retrieve가 fresh 페이지를 만들고 리포트가 안 뜬다. retrieve는 fire
  페이지 객체를 우선 사용한다(`firePage`). URL 프래그로 찾지 말 것 — 전송 직후 URL이 `WEB:<uuid>`
  임시값이라 실제 `/c/6a8e…`와 안 맞는다.
- **Chat vs Work 모드**: 계정이 "Work"로 튀면 심층 리서치가 사라지고 5.6 Sol이 .md 파일을 뱉는다.
  매 fire에서 Chat 강제 + Deep research 메뉴 존재를 검증한다.
- **citation 링크 탭 폭증**: 좌표 클릭이 리포트 각주 링크를 눌러 새 탭이 수십 개 열린다.
  `ctx.on('page', pg=>{ 외부 URL이면 pg.close() })`로 자동 닫는다.
- **레이트리밋**: 빠른 반복 조작에 "Too many requests"로 대화 접근이 차단된다. 느린 페이스
  (첫 회수 13분 후·간격 8분·재시도 6회), "Too many requests" 감지 시 30분 백오프. 심하면
  ~90분 식힌 뒤 재개. 재개 전 새 채팅에서 `ratelimited:false & deepresearch:true`를 한 번만 확인.
- **재개 가능**: batch.cjs는 `<label>-final.md`가 있으면 그 도메인을 skip한다. 죽으면 그냥 다시 실행.
- **배포**: 데몬 소스(순서·심층리서치 모드)를 고치면 이미지 재빌드 + `docker stop -t 40 gwp-slot-<x>`
  후 재생성해야 반영된다(위 "배포 함정" 절). 단, 이 배치 하네스는 데몬이 아니라 diag 컨테이너를 쓴다.

## 실행·재개

```bash
# (최초 1회) 프로필 사본 + 컨테이너
cp -a ~/.local/state/gpt-webai-pro/slots/slot-a/profile ~/rf_work/diag/profile
docker run -d --name gwp-diag \
  --mount type=bind,src=$HOME/rf_work/diag/profile,dst=/profile \
  --mount type=bind,src=$HOME/rf_work/diag/outbox,dst=/outbox \
  --mount type=bind,src=$HOME/rf_work/diag,dst=/work,readonly \
  --shm-size=1g --entrypoint bash home-server/gpt-webai-pro-slot:latest /work/diag-entry.sh
# /work 에 batch.cjs·diag-entry.sh·프롬프트(l??_prompt.md)·routefork_context.zip 배치
# 배치 실행(재개 가능)
setsid nohup bash -c "docker exec gwp-diag node /work/batch.cjs > ~/rf_work/batch_driver.log 2>&1" </dev/null >/dev/null 2>&1 &
# 진행: ~/rf_work/diag/outbox/batch.log , 회수물: ~/rf_work/diag/outbox/<label>-final.md
```


## 회수 결함 수정 기록 (2026-09-01, home-server-infra ee855a3a)

증상: run이 5분 관찰 창 뒤 `send_uncertain`을 찍고 종료 → 10분 주기 `gpt-webai-pro-reap.timer`가 owner.lock을 잡고 reconcile·완료(answer.md 저장) → 그 사이 호출자의 `resume`는 락 경합으로 즉시 `running`("소유 프로세스 생존")만 반환 → 완료가 어떤 envelope 파일에도 안 떨어짐.
수정: (1) `continue()` attach-대기(`waitForOwnerLock`, GWP_OWNER_ATTACH_POLL_MS=2000) — 종료 상태면 envelope 반환, 락 해제 시 이어받음, timeout에만 running. (2) uncertain 직후 인라인 reconcile 최대 GWP_INLINE_RECONCILE_TRIES=3회, 백오프 GWP_INLINE_RECONCILE_BACKOFF_MS=20000. 단위 테스트 test/unit/owner-attach.test.ts.
운영 규칙: envelope 파일의 `running`은 신뢰하지 말고 요청 디렉토리 상태 또는 재-resume로 판정. 폴링은 터미널 상태 기준.
