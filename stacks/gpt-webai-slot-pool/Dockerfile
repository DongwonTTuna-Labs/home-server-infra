FROM node:24-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      chromium \
      curl \
      dumb-init \
      fonts-liberation \
      procps \
      xvfb \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g agbrowse@latest

COPY scripts/slot-entrypoint.sh /usr/local/bin/gpt-webai-slot-entrypoint
COPY scripts/slot-healthcheck.sh /usr/local/bin/gpt-webai-slot-healthcheck
RUN chmod 0755 /usr/local/bin/gpt-webai-slot-entrypoint /usr/local/bin/gpt-webai-slot-healthcheck

ENV CHROME_BINARY_PATH=/usr/bin/chromium
ENV CHROME_NO_SANDBOX=1
ENV DISPLAY=:99
ENV AGBROWSE_WEB_AI_AUTO_START=0

ENTRYPOINT ["dumb-init", "--", "gpt-webai-slot-entrypoint"]
