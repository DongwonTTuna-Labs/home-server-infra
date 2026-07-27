# gpt-webai-pro

`gpt-webai-pro`는 ChatGPT **Pro Extended** 웹 세션에 프롬프트와 첨부를 보내고, 완료된
답변과 ChatGPT가 렌더한 다운로드 파일을 로컬 state directory에 보존하는 슬롯 풀입니다.
Thinking/xhigh 경로와 다른 모델 fallback은 제공하지 않습니다.

## 설치

요구 사항은 Node.js 22 이상, Docker, util-linux의 `flock`, 그리고 슬롯별로 로그인된
Chromium profile입니다. request별 `send.lock`은 kernel flock으로만 잠기며 프로세스 종료 시
커널이 자동 해제합니다.

```bash
cd stacks/gpt-webai-pro
npm install
npm run build
ln -sfn "$PWD/bin/gpt-webai-pro" ~/.local/bin/gptpro
```

기본 state root는
`${GPT_WEBAI_PRO_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-pro}`입니다.
이미지 이름과 슬롯/계정/loopback 포트 매핑은 `config/slots.json`, UI의 Intelligence 라벨과
단일 `Pro` 목표는 `config/labels.json`에서 관리합니다.

컨테이너 이미지는 다음과 같이 빌드합니다.

```bash
docker build -f container/Dockerfile -t home-server/gpt-webai-pro-slot:latest .
```

## 로그인 profile 시딩

슬롯 컨테이너와 시딩용 Chromium을 동시에 실행하지 마십시오. 가장 간단한 컷오버는 기존
로그인 profile을 슬롯별 `slots/<slotId>/profile/`로 복사하는 것입니다. 새로 로그인할 때는
해당 컨테이너가 정지된 상태에서 host Chromium으로 같은 디렉토리를 엽니다.

```bash
state="${GPT_WEBAI_PRO_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-pro}"
mkdir -p "$state/slots/slot-01/profile"
chromium --user-data-dir="$state/slots/slot-01/profile" https://chatgpt.com/
```

로그인을 마치고 Chromium을 완전히 종료한 뒤 `gpt-webai-pro cleanup --apply`를 실행합니다.
이 명령은 `needs_login` 슬롯의 daemon `readiness()`를 다시 확인하며, 실제로 `ready`인 슬롯만
`idle`로 되돌립니다. 슬롯 daemon은 host `127.0.0.1`의 고정 포트에만 노출되고 컨테이너를
기동할 때마다 `slots/<slotId>/daemon.token`이 mode 0600으로 교체됩니다. 쿠키, token,
profile 파일은 로그나 첨부로 내보내지 마십시오.

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
```

`run`, `resume`, `release`는 항상 JSON request envelope 하나를 stdout에 출력합니다. 긴 생성의
timeout은 실패가 아니라 `status:"running"`이며, 같은 `sessionId`의 `resumeCommand`를
사용해야 합니다. 전송 여부가 불확실한 요청은 즉시 다시 보내지 않고 `resume`의 read-only
reconcile을 거칩니다. 살아 있는 sender와 동시에 `resume`하면 상태를 바꾸지 않고
`status:"running"`, message `전송 진행 중(소유 프로세스 생존)`을 반환합니다.

`status`는 슬롯 state/cooldown/최근 사용 시각과 비종결 요청을 자체 JSON으로 출력합니다.
`cleanup`은 기본이 dry-run이며, `--apply`일 때만 stale slot, 오래된 고아 컨테이너, 로그인
복구를 실제로 처리합니다. `release`는 요청을 `failed`로 강제 종결하고 더 이상 active 요청이
없는 슬롯 컨테이너를 정지합니다.

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

- `needs_login`: 해당 account의 profile을 다시 시딩한 뒤 `cleanup --apply`를 실행합니다.
- `provider_limit`: 슬롯은 3분 cooldown에 들어갑니다. 기존 session을 나중에 `resume`합니다.
- `pool_busy`: queue daemon은 없습니다. `nextCommand`의 동일-session `resume`을 호출합니다.
- `model_unavailable`: composer의 Intelligence picker에 `Pro` 라디오가 실제로 보이는지와
  `labels.json`을 확인합니다.
- `send_uncertain`: 새 `run`을 만들지 말고 반드시 기존 session을 `resume`합니다. reconcile도
  증명하지 못하면 ChatGPT의 열린 탭을 사람이 확인해야 합니다.
- `daemon_unreachable`: `docker inspect gwp-slot-01`, `config/slots.json`의 포트,
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
