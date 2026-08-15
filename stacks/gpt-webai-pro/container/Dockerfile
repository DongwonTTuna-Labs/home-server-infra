FROM node:24-bookworm-slim AS build

# better-sqlite3 native build (daemon은 미사용이지만 단일 package.json이라 설치는 됨)
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 make g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json ./
COPY src ./src
RUN npm run build && npm prune --omit=dev

FROM node:24-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    DISPLAY=:99 \
    GWP_CDP_URL=http://127.0.0.1:9222 \
    GWP_OUTBOX_DIR=/outbox

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      chromium \
      curl \
      dumb-init \
      fonts-liberation \
      novnc \
      websockify \
      x11vnc \
      xvfb \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/dist ./dist
COPY --from=build /app/node_modules ./node_modules
COPY package.json ./
COPY config ./config
COPY container/entrypoint.sh /usr/local/bin/gpt-webai-pro-entrypoint
RUN chmod 0755 /usr/local/bin/gpt-webai-pro-entrypoint \
    && find /app -type d -exec chmod 0755 {} + \
    && find /app -type f -exec chmod 0644 {} +

# --single-child: 시그널을 entrypoint에게만 전달한다. 기본(프로세스 그룹 전파)이면
# Chrome이 TERM을 직접 받아 비플러시 종료 → 세션 쿠키 유실 (entrypoint.sh 주석 참조).
ENTRYPOINT ["dumb-init", "--single-child", "--", "gpt-webai-pro-entrypoint"]
