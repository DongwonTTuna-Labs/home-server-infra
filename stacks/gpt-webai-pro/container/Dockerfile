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

ENTRYPOINT ["dumb-init", "--", "gpt-webai-pro-entrypoint"]
