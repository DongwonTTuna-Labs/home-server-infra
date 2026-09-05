# gpt-webai-pro

`gpt-webai-pro`는 ChatGPT **Pro Extended** 웹 세션에 프롬프트와 첨부를 보내고, 완료된
답변과 ChatGPT가 렌더한 다운로드 파일을 로컬 state directory에 보존합니다. 슬롯 하나는
계정 하나이자 Chromium profile 하나이며, 슬롯 안에서는 요청별 탭으로 최대 3개 요청을
동시에 처리합니다. 일반 텍스트 위임은 Pro이며, `image-batch`는 전용 Xhigh 이미지 생성 경로입니다. 다른 모델로 자동 대체하지 않습니다.

## 설치

요구 사항은 Node.js 22 이상, Docker, util-linux의 `flock`, 그리고 슬롯별로 로그인된
Chromium profile입니다. request별 `send.lock`/`owner.lock`과 슬롯 runtime lock은 kernel
flock으로만 잠기며 프로세스 종료 시 커널이 자동 해제합니다.

macOS의 `flock`은 Homebrew `util-linux` keg에 포함되어 있으므로 최초 한 번 설치하고
실행 경로에 연결합니다. Docker container smoke는 Docker Desktop의 host networking 기능을
요구하지 않습니다.

```bash
brew install util-linux
mkdir -p ~/.local/bin
ln -sfn "$(brew --prefix util-linux)/bin/flock" ~/.local/bin/flock
```

```bash
cd stacks/gpt-webai-pro
npm install
npm run build
mkdir -p ~/.local/bin
ln -sfn "$PWD/bin/gpt-webai-pro" ~/.local/bin/gpt-webai-pro
ln -sfn "$PWD/bin/gpt-webai-pro" ~/.local/bin/gptpro
```

기본 state root는
`${GPT_WEBAI_PRO_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-pro}`입니다.
이미지 이름, 슬롯당 동시성, 슬롯/계정/loopback 포트 매핑은 `config/slots.json`, UI의
Intelligence 라벨과 일반 텍스트 요청의 `Pro` 목표는 `config/labels.json`에서 관리합니다. 기본 구성은
`slot-a`, `slot-b`, `slot-c`, `slot-d` 네 계정과 포트 19301–19304입니다.

별도 작업본을 병렬로 실행할 때는 state root뿐 아니라 슬롯 ID와 daemon 포트도 분리합니다.
컨테이너 이름은 `gwp-<slotId>`이므로 state root만 바꾸면 기존 컨테이너와 충돌합니다.
supervisor는 `/profile`, `/inbox`, `/outbox`의 bind source를 현재 state의 canonical path와
비교하고, 다르거나 확인할 수 없으면 토큰 교체·기동·정지·삭제 없이 거부합니다.
컨테이너를 조작할 때는 소유권을 확인한 Docker ID를 사용합니다. 충돌한 컨테이너를 임의로
삭제하거나 다른 작업의 profile·token을 복사하지 마세요.

컨테이너 이미지는 다음과 같이 빌드합니다.

```bash
docker build -f container/Dockerfile -t home-server/gpt-webai-pro-slot:latest .
```

## Xhigh 이미지 배치

`image-batch`는 별도 유료 API 없이 ChatGPT 웹 이미지 도구로 프롬프트당 한 장을 생성합니다.
청크마다 새 대화를 만들며, 한 청크는 최대 5장이고 순차 처리합니다. 9개 프롬프트는 5+4 청크가 됩니다.
생각 강도는 **Pro 바로 아래 Xhigh**입니다. 구 UI에서는 `Extra High`, GPT-6 UI에서는
최대값에서 한 칸 아래 `Extended`를 선택하고 표시 라벨을 재검증합니다.
일반 `run`의 Pro 선택은 그대로이며 폐기한 `gptxhigh` 실행기를 재사용하지 않습니다.

```json
{"images":[{"id":"map-home","prompt":"첫 화면의 전체 제작 지시"},{"id":"itinerary","prompt":"두 번째 화면의 전체 제작 지시"}]}
```

ID는 영숫자로 시작하는 1–80자의 영숫자·하이픈·밑줄이며 대소문자를 무시해 유일해야 합니다.
전체 제작 근거와 프롬프트 목록을 ZIP으로 묶고 내용 설명을 함께 넣으십시오. 첨부는 외부로
업로드되므로 자격증명·개인정보를 포함하지 않습니다.

```bash
gpt-webai-pro image-batch --manifest ./images.json --file ./context.zip --timeout-seconds 600
gpt-webai-pro image-batch --batch img_0123456789abcdef --timeout-seconds 600
# 전송 없이 입력만 영속화하고 배치 ID를 받기
gpt-webai-pro image-batch --manifest ./images.json --file ./context.zip --timeout-seconds 0
```

배치 JSON은 `batchId`, `status`, `resumeCommand`, `expectedImages`, `downloadedImages`,
`chunks`(각 청크의 기존 request envelope), `artifacts`를 반환합니다. 사용법 오류는 다른 관리
명령과 같이 `{ok:false,error}`와 exit 2입니다. 타임아웃·모호한 전송은 새 배치를 만들지 말고
반환된 **배치 ID**로 재개합니다. 이미 끝난 청크는 건너뛰며, 전송 후 중단된 요청은 기존
앵커로 확인합니다. 다운로드 실패는 같은 응답에서 다시 받으며 생성 프롬프트를 다시 보내지 않습니다.

이미지 파일은 실제 브라우저 다운로드 이벤트로 받습니다. PNG/JPEG/WebP의 전체 픽셀이
디코딩되고 각 변이 256px 이상이어야 저장합니다. 예상 수량과 저장 수량이 모두 맞아야
완료입니다. 누락·다운로드 실패는 `image_incomplete`로 남기고 받은 파일은 보존합니다. 브라우저 연결 오류도 같은 요청으로 재개합니다.
`artifacts[].promptId`와 파일명은 **응답 카드 순서**를 입력 순서에 대응시킨 값입니다.
모델이 순서를 바꾸거나 여러 화면을 한 장에 그리면 수량 검사로 의미를 보장할 수 없으므로
실제 이미지를 열어 콘텐츠·순서·한국어 표기를 확인해야 합니다.

### 컨테이너 화면 진단

```bash
gpt-webai-pro inspect --slot slot-a --open-tools
gpt-webai-pro inspect --slot slot-a --select-image-tool
gpt-webai-pro inspect --slot slot-a --image-prompt-file ./prompt.md
gpt-webai-pro inspect --session req_0123456789abcdef
gpt-webai-pro inspect --session req_0123456789abcdef --open-image-index 0
```

슬롯 진단은 빈 새 탭에서 도구 선택·입력을 관찰하고 닫으며 전송하지 않습니다.
세션 진단은 지정 요청의 대화만 엽니다. `--open-image-index`는 0부터 시작하는 원본 뷰어를 관찰한 뒤 닫습니다. 활성 요청이 있는 슬롯에는 세션 ID를 지정합니다.
`outbox/diagnostics/`의 스크린샷과 접근성·컨트롤 기록에는 본문과 편집기 내용이 포함됩니다.
사이드바·프로필·쿠키·이미지 원본 URL은 수집하지 않습니다. 컨테이너에는 한국어 표시를 위한 Noto CJK 글꼴을 포함합니다.
구성 실험에는 `GWP_CONFIG_PATH`와 별도 `GPT_WEBAI_PRO_STATE_DIR`를 쓸 수 있습니다.
동일 프로필을 두 런타임에서 동시에 열거나 프로필을 복사하지 마십시오.

## 계정 로그인 시딩

각 계정은 자기 슬롯의 영속 profile 하나만 사용합니다. profile을 다른 슬롯으로 복제하거나
같은 계정을 여러 profile에 로그인하지 마십시오. rotating refresh token 때문에 사본들이
서로를 로그아웃시킬 수 있습니다.

비종결 요청이 없는 슬롯을 골라 로그인 모드를 시작합니다. 기본 슬롯의 noVNC 주소는 각각
`slot-a`=19901, `slot-b`=19902, `slot-c`=19903, `slot-d`=19904입니다.

```bash
gpt-webai-pro login --slot slot-a
# 안내된 http://127.0.0.1:19901/vnc.html 을 로컬 브라우저에서 연다.
```

noVNC 화면에서 사람이 직접 ChatGPT 자격증명, 2FA, CAPTCHA를 처리합니다. CLI는 5초마다
readiness를 확인해 최대 15분 기다립니다. `ready`가 되면 컨테이너를 정지하고 슬롯을
`idle`로 기록합니다. 타임아웃이나 Ctrl-C로 중단해도 로그인 컨테이너는 정지되며, 명령은
noVNC URL·대기 경과는 stderr로 스트리밍하고 stdout에는 최종 JSON 한 객체만 출력합니다.
성공은 exit 0, 타임아웃과 daemon 오류도 관찰 결과이므로 exit 0, SIGINT/SIGTERM은 정리 후
exit 130, 잘못된 슬롯이나 active 요청이 있는 사용법 오류는 exit 2입니다. 계정마다
`slot-b`, `slot-c`, `slot-d`에도 같은 절차를 반복합니다.

```json
{"ok":true,"slot":"slot-a","state":"ready","novncUrl":"http://127.0.0.1:19901/vnc.html"}
```

noVNC는 로그인 모드에서만 기동되고 host loopback에만 publish되며 VNC 비밀번호가 없습니다.
외부 인터페이스로 노출하거나 인증 없는 포트 포워딩을 하지 마십시오. daemon도 host
`127.0.0.1`의 고정 포트에만 노출되고, 컨테이너를 기동할 때마다
`slots/<slotId>/daemon.token`이 mode 0600으로 교체됩니다. 쿠키, token, profile 파일은
로그나 첨부로 내보내지 마십시오.

## 사용

```bash
gptpro "질문"
gptpro run --file ./context.txt "이 파일을 검토해 줘"
gptpro run --prompt-file ./prompt.md --timeout-seconds 10800
gptpro resume --session req_0123456789abcdef
gptpro status --json
gptpro cleanup --dry-run
gptpro cleanup --apply
gptpro release --session req_0123456789abcdef
gpt-webai-pro keepalive
gpt-webai-pro reap
```

`run`, `resume`, `release`는 항상 JSON request envelope 하나를 stdout에 출력합니다. 긴 생성의
timeout은 실패가 아니라 `status:"running"`이며, 같은 `sessionId`의 `resumeCommand`를
사용해야 합니다. CLI owner가 끝나면 같은 슬롯에 다른 live owner가 없는 Chromium
runtime은 정지하지만 SQLite 요청·conversation URL·영속 profile은 보존되므로 다음
`resume`이 그대로 이어갑니다. 전송 여부가 불확실한 요청은 즉시 다시 보내지 않고
`resume`의 read-only reconcile을 거칩니다. 살아 있는 run/resume 또는 reaper와 동시에
호출하면, 다른 소유자의 DB 상태나 runtime을 바꾸지 않고 호출 시간 예산 안에서 기다립니다.
소유자가 완료하면 저장된 envelope를 반환하고, 먼저 lock을 놓으면 남은 예산으로 이어받습니다.
예산이 끝나도록 점유 중이면 `status:"running"`, message
`전송 진행 중(소유 프로세스 생존)`을 반환합니다. `release`는 대기하지 않습니다.

전송 후 `send_uncertain`이 발생해도 시간 예산이 충분하면 같은 호출에서 read-only
reconcile을 추가로 시도합니다. 기본 최대 3회, 간격 20초이며 각 대기 전에 간격의 두 배
이상이 남아 있어야 합니다. 횟수나 예산을 소진하면 `uncertain` 상태와 같은 세션의
`resumeCommand`를 보존합니다. 반복 관측만으로 재전송을 허용하지 않으며 기존의 명시적인
부재 증명 규칙만 attempt 2를 허용합니다. 설정은 호출 시작 시 읽습니다.

| 환경 변수 | 기본값 | 의미 |
|---|---|---|
| `GWP_OWNER_ATTACH_POLL_MS` | 2000 | owner 결과·lock 확인 간격, 남은 대기 예산으로 제한 |
| `GWP_INLINE_RECONCILE_TRIES` | 3 | 추가 reconcile 횟수, 0이면 비활성; 잘못된 값은 기본값 |
| `GWP_INLINE_RECONCILE_BACKOFF_MS` | 20000 | 추가 reconcile 전 대기 간격 |

### 소스 통합과 운영 배포

PR 머지는 저장소의 소스 통합이며 운영 배포가 아닙니다. 호스트 CLI/supervisor는
실행 링크가 가리키는 작업본의 `dist`를 쓰고, 브라우저 daemon은 Docker 이미지 안의
`/app/dist`를 쓰므로 각각의 빌드가 다를 수 있습니다. `git pull`이나 호스트 빌드만으로
daemon이 교체되지는 않습니다. 배포는 별도 승인 후 진행하고, 운영 작업본의 미커밋 변경을
보존한 채 검증된 동일 소스로 호스트와 이미지를 준비해야 합니다. 미배포 심층 리서치
수정과 `--deep-research` 옵션은 이 통합 범위에 포함하지 않습니다.

이미지 배치는 주간 사용량 원장을 보존하면서 DB를 v4로 이관합니다. 이 코드를 기존 운영
state에 실행하면 DB가 갱신되며 구 v2/v3 호스트는 v4 DB를 열 수 없으므로, 운영 실행 전 DB·profile을
안전하게 보존하고 롤백 절차까지 확인해야 합니다. PR 검증에는 격리된 테스트 state만 씁니다.

`status`는 슬롯 state/cooldown/최근 사용 시각/`activeRequests`와 비종결 요청을 자체 JSON으로
출력합니다. 슬롯마다 주간 사용량 `weeklyUsed`(최근 7일 확정 전송 수), `weeklyLimit`
(`config/slots.json`의 `weeklyLimit`, 슬롯별 `weeklyLimit`이 있으면 그것, 없으면 null=무제한),
`weeklyResetAt`(한도에 닿았을 때 가장 오래된 전송이 7일 창 밖으로 나가 1건이 풀리는 시각, 그 외 null)도
함께 나옵니다.

## 모델 보장과 주간 한도

전송 직전 daemon은 composer 알약이 `labels.json`의 `target`(기본 `Pro`)인지 보장합니다.
2026-09 GPT-6 UI에서는 알약이 "6 / Pro"(모델 버전 + 생각 강도)이고, 메뉴 안에 생각 강도
슬라이더(최대 = Pro)와 "Select model" 라디오(Latest / GPT-5.6 Sol / GPT-5.5)가 있습니다.
2026-09-05 관찰된 UI에서는 `[role=slider]`가 숨김·`tabindex=-1` 상태이며, 보이는 `Power`
menuitem이 키보드 입력을 받습니다. daemon은 유일한 슬라이더에서 상태를 읽되 보이는 입력점에
`ArrowRight`를 한 단계씩 보내고 매번 `aria-valuenow` 변경을 확인합니다. 직접 입력 가능한
슬라이더도 지원하며 `Home`/`End` 지원은 가정하지 않습니다. 최댓값의 상태 문구가 `Pro`인지,
`labels.json`의 `modelVersion`(기본 `Latest` = 6)이 선택돼 있는지
확인한 뒤 메뉴를 닫고 알약을 다시 읽어 검증합니다. 구 UI(단일 Intelligence 라디오)도 그대로
지원합니다. 어느 UI에서도 다른 강도·모델로 대체하지 않으며, 목표를 만들 수 없으면
`model_unavailable`입니다. 보장된 라벨(예: `6 Pro`)은 send 결과의 `modelLabel`로 돌아옵니다.

버전 선택으로 메뉴가 닫히면 다시 열어 목표 라디오의 `aria-checked=true`를 확인합니다.
새 UI를 이미 알약에서 관찰했다면 Power나 버전 컨트롤이 없어도 구 UI로 간주하지 않습니다.
확인 대상이 사라지거나 선택되지 않은 상태에서는 `pre_click` 실패로 끝나며 전송하지 않습니다.
구 UI의 `Instant / 5.5` 표시만으로 새 UI라고 단정하지는 않습니다. 실제 `Pro` 라디오 선택과
`aria-checked`를 확인한 경우에는 기존 단일 Intelligence 경로로 처리합니다.

ChatGPT Pro 계정의 주간 Pro 한도(현재 200회)는 SQLite `usage_events`에 계정(슬롯)별로 기록해
관리합니다. 전송이 확정(confirmed/reconciled)될 때 요청당 1건을 남기고, 최근 7일 이동창 안의
건수가 `weeklyLimit`에 닿은 슬롯은 새 요청 할당에서 제외됩니다. 쓸 수 있는 슬롯이 전부 한도에
닿으면 envelope는 `recovering` / `errorKind: "weekly_limit"`이며 `message`에 가장 이른 리셋
시각을 적습니다. 기존 session은 그 뒤 `resume`으로 이어갑니다. 제공자의 정확한 리셋 시각은
공개돼 있지 않으므로 7일 이동창은 보수적 근사입니다. `GWP_ONLY_SLOT`으로 고정한 슬롯이 한도에
닿아도 다른 슬롯으로 넘어가지 않습니다.
`cleanup`은 기본이 dry-run이며, `--apply`일 때만 live CLI owner가 없는 managed runtime
정지와 `needs_login` 재검사를 실제로 처리합니다. 요청/profile/쿠키는 삭제하지 않습니다.
`release`는 요청을 `failed`로 강제 종결하고 live owner가 없는 슬롯 runtime을 정지합니다.

## 방치 요청 reaper

`reap`은 이미 전송된 `sending`/`generating`/`uncertain` 요청 중 가장 오래 기다린 한 건만
기본 120초 동안 resume합니다. 실행 뒤 해당 행의 순번을 뒤로 보내 다음 timer tick이 다른
요청을 고르므로 요청 수에 비례해 한 번의 서비스가 길어지지 않습니다. `staged`는 전송을
새로 시작할 수 있으므로 자동 reaper가 건드리지 않으며, 오래됐다는 이유로 요청을 자동
실패시키지도 않습니다. 호출이 끝난 뒤 owner 없는 runtime은 정지합니다.

timer는 부팅 5분 후 첫 실행하고, 서비스가 끝난 시점부터 10분 뒤 다음 실행을 예약합니다.
서비스 상한은 5분입니다.

## 로그인 keepalive

`keepalive`는 비종결 요청이 없는 모든 구성 슬롯을 차례로 기동해 `readiness`를 읽습니다.
페이지 로드가 세션 token을 갱신하고, `ready`/`needs_login`/`provider_limit` 확정 관찰만 각각
슬롯의 `idle`/`needs_login`/`provider_limit` 상태에 반영됩니다. `unknown`이나 기동·RPC 오류는
기존 DB 상태를 보존합니다. 비종결 요청이 있는 슬롯은 요청 resume/reaper가 관리하므로
건너뛰며, 원래 꺼져 있던 컨테이너는 검사 후 다시 정지합니다. `state`는 최종 DB 상태, `probe`는 이번 관찰
결과입니다. `needs_login`과 `unreachable`도 운영 관찰이므로 명령 exit code는 항상 0입니다.

```json
{"ok":true,"slots":[{"id":"slot-a","state":"idle","probe":"ready"},{"id":"slot-b","state":"needs_login","probe":"needs_login"},{"id":"slot-c","state":"provider_limit","probe":"unreachable"}]}
```

keepalive user timer는 매일 09:20 이후 최대 10분의 무작위 지연을 두며, 놓친 실행은 user
manager가 다시 시작될 때 보충합니다. 아래에서 keepalive와 reaper timer를 함께 설치합니다.

```bash
cd stacks/gpt-webai-pro
install -Dm0644 systemd/gpt-webai-pro-keepalive.service \
  ~/.config/systemd/user/gpt-webai-pro-keepalive.service
install -Dm0644 systemd/gpt-webai-pro-keepalive.timer \
  ~/.config/systemd/user/gpt-webai-pro-keepalive.timer
install -Dm0644 systemd/gpt-webai-pro-reap.service \
  ~/.config/systemd/user/gpt-webai-pro-reap.service
install -Dm0644 systemd/gpt-webai-pro-reap.timer \
  ~/.config/systemd/user/gpt-webai-pro-reap.timer
systemctl --user daemon-reload
systemctl --user enable --now gpt-webai-pro-keepalive.timer gpt-webai-pro-reap.timer
systemctl --user start gpt-webai-pro-keepalive.service  # 선택적 즉시 점검
systemctl --user list-timers gpt-webai-pro-keepalive.timer gpt-webai-pro-reap.timer
```

사용자가 로그아웃한 뒤에도 user manager가 살아 있어야 하는 호스트라면 운영자가 별도로
`loginctl enable-linger "$USER"`를 설정합니다. 실행 결과와 오류는 다음으로 확인합니다.

```bash
systemctl --user status gpt-webai-pro-keepalive.timer
journalctl --user -u gpt-webai-pro-keepalive.service -n 100 --no-pager
systemctl --user status gpt-webai-pro-reap.timer
journalctl --user -u gpt-webai-pro-reap.service -n 100 --no-pager
```

## 저장 결과

요청별 주요 파일은 다음과 같습니다.

```text
requests/<reqId>/prompt.md
requests/<reqId>/attachments/
requests/<reqId>/answer.md
requests/<reqId>/artifacts/
requests/<reqId>/failure/
requests/<reqId>/log.jsonl
```

SQLite `db.sqlite`가 유일한 상태 진실입니다. `log.jsonl`은 사람용 진단 기록일 뿐 복구에
사용하지 않습니다. artifact control은 두 번까지 시도하며, 일부 다운로드가 끝내 실패해도
성공한 파일과 답변은 보존하고 요청은 `complete`로 종결합니다. 실패 label은 envelope의
`message`에 기록됩니다.

## 진단

- `needs_login`: `gpt-webai-pro login --slot <slotId>`로 해당 계정을 다시 로그인합니다.
- `login`이 active 요청 때문에 거부됨: `status --json`의 해당 슬롯 `activeRequests`와 비종결
  요청을 확인하고 완료·`resume`·명시적 `release` 후 다시 실행합니다.
- noVNC 페이지에 접속할 수 없음: `login` 프로세스가 아직 실행 중인지와 안내된
  127.0.0.1 포트가 해당 슬롯 포트+600인지 확인합니다. noVNC는 일반 run 컨테이너에서는
  기동되지 않습니다.
- keepalive가 실행되지 않음: user timer의 다음 실행 시각과 journal을 위 명령으로 확인하고,
  로그아웃 중 실행이 필요하면 user lingering 상태를 확인합니다.
- `provider_limit`: 슬롯은 3분 cooldown에 들어갑니다. 기존 session을 나중에 `resume`합니다.
- `pool_busy`: 세 슬롯이 동시성 한도에 찼거나 할당 불가 상태입니다. queue daemon은 없으므로
  `nextCommand`의 동일-session `resume`을 호출합니다.
- `model_unavailable`: 알약을 열었을 때 생각 강도 슬라이더가 최대(Pro)로 올라가는지, "Select
  model"에 `labels.json`의 `modelVersion` 라디오가 있는지(구 UI라면 `Pro` 라디오가 보이는지)와
  `labels.json`을 확인합니다.
- `weekly_limit`: 쓸 수 있는 모든 계정이 주간 한도에 닿았습니다. `status --json`의
  `weeklyUsed`/`weeklyResetAt`을 보고 리셋 뒤 같은 session을 `resume`합니다.
- `send_uncertain`: 새 `run`을 만들지 말고 반드시 기존 session을 `resume`합니다. reconcile도
  증명하지 못하면 ChatGPT의 열린 탭을 사람이 확인해야 합니다.
- `daemon_unreachable`: `docker inspect gwp-slot-a`, `config/slots.json`의 포트,
  `slots/<slotId>/daemon.token`의 존재와 mode 0600, 컨테이너의
  `/tmp/gwp-runtime/{chromium,xvfb}.log`를 확인합니다. 토큰 값 자체는 출력하지 마십시오.
- 직접 DNS/offline 증거가 있는 `network_disconnected`만 exit 1입니다. 나머지 provider,
  auth, browser 상태를 네트워크 단절로 분류하지 않습니다.

로컬 검증은 라이브 ChatGPT에 접촉하지 않습니다.

```bash
npm run build
npm test
```

`scripts/container-smoke.sh`는 Docker image와 로컬 fake-chatgpt를 연결하는 수동 릴리스
검증입니다. 라이브 1회 왕복은 별도 컷오버 단계에서만 `GWP_LIVE=1 gpt-webai-pro smoke`로
실행합니다.
