# Google Workspace CLI (`gws`) — 구글 작업 표준 경로

**구글(Gmail·Drive·Calendar·Sheets·Docs 등)에 관한 모든 작업은 `gws` CLI로 한다**
(https://github.com/googleworkspace/cli). claude.ai Gmail MCP 커넥터, 브라우저 자동화,
개별 API 직접 호출은 gws가 안 될 때의 보조 수단일 뿐이다.

## 설치·인증 상태 (2026-09-05 재확인)

- 설치: npm 전역 `@googleworkspace/cli` (`gws` 명령, v0.22.5). 업데이트: `npm i -g @googleworkspace/cli`.
- GCP 프로젝트: `gws-ttuna0790-260607`, OAuth 클라이언트 구성 완료 (`~/.config/gws/client_secret.json`).
- 계정: **ttuna0790@gmail.com**, `gws auth login --full`로 전체 스코프 동의 완료
  (drive·spreadsheets·gmail.modify·calendar·documents·presentations·tasks·pubsub·cloud-platform).
- 자격증명은 키링 암호화 저장. 상태 확인: `gws auth status` (`token_valid` 확인).
  2026-09-05 실측: `token_valid: true`, refresh token 보유, 스코프 14개, 활성 API 47개.
- 토큰 만료/철회 시: `gws auth login --full` 재실행 → 브라우저 인증. **CDP 자동화 Chrome에서는
  구글이 로그인을 차단**("브라우저가 안전하지 않을 수 있습니다") — 반드시 사용자의 일반
  브라우저에서 인증 URL을 열게 한다. 콜백은 localhost 포트로 gws가 수신.

## 사용법 핵심 (함정 포함)

1. **resource 계열 명령은 경로 파라미터를 반드시 `--params` JSON으로** 준다. 개별 플래그
   (`--user-id`, `--id`)는 없다. **`userId` 누락 시 조용히 실패**하고, 파이프라인에서
   `2>/dev/null`로 가리면 "0건"처럼 보인다 — 검색 결과 0건이 나오면 가장 먼저 의심할 것.
   ```bash
   gws gmail users messages list --params '{"userId":"me","q":"newer_than:2d 검색어","maxResults":10}' --format json
   gws gmail users messages get  --params '{"userId":"me","id":"<ID>","format":"full"}' --format json
   gws gmail users getProfile    --params '{"userId":"me"}'   # 서브커맨드는 camelCase
   ```
2. 헬퍼 명령은 플래그식: `gws gmail +triage`(안읽음 요약), `+read --id <ID>`, `+send`,
   `+reply`, `+watch`. 단 `+read`는 From 헤더 없는 메일(일부 광고)에서 500을 내니 그 경우
   resource `get`으로 직접 읽고 base64url 디코드한다.
3. 본문 디코드: `payload.parts[].body.data`는 **base64url** — python `base64.urlsafe_b64decode`.
   text/plain 우선, 없으면 html 태그 제거.
4. 출력: `--format json`이 기본이지만 stderr에 "Using keyring backend" 라인이 섞인다 —
   파싱 시 `2>/dev/null` 또는 정규식으로 JSON만 추출.
5. 메일 발송(`+send`·`+reply`)은 대외 발신이므로 **보내기 전 사용자 확인**을 받는다.

## 스코프 정책

- 현재 `--full` (gmail.modify 포함 — 읽기·라벨·발신 가능, 영구삭제 불가).
- 축소가 필요하면 `gws auth login -s gmail --readonly` 식으로 재로그인.
- 새 스코프가 필요한 API 호출이 403이면 스코프 목록 확인 후 재로그인으로 해결한다 —
  client_secret이나 GCP 프로젝트는 건드리지 않는다.
