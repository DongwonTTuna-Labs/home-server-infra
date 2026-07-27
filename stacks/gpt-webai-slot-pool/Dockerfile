FROM node:24-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      chromium \
      curl \
      dumb-init \
      fonts-liberation \
      python3 \
      procps \
      xvfb \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p \
      /usr/local/lib/gpt-webai-slot-pool/contracts/ui-labels-r14 \
      /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright

COPY provider/chatgpt-playwright/package.json provider/chatgpt-playwright/package-lock.json /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright/
RUN cd /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright \
    && npm ci --omit=dev --ignore-scripts

COPY provider/chatgpt-playwright /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright
COPY contracts/ui-labels-r14 /usr/local/lib/gpt-webai-slot-pool/contracts/ui-labels-r14
COPY scripts/slot-entrypoint.sh /usr/local/bin/gpt-webai-slot-entrypoint
COPY scripts/slot-healthcheck.sh /usr/local/bin/gpt-webai-slot-healthcheck
RUN chmod 0755 /usr/local/bin/gpt-webai-slot-entrypoint /usr/local/bin/gpt-webai-slot-healthcheck \
    && find /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright -type d -exec chmod 0755 {} + \
    && find /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright -type f -exec chmod 0644 {} + \
    && chmod 0755 /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright/cli.mjs \
    && find /usr/local/lib/gpt-webai-slot-pool/contracts -type d -exec chmod 0755 {} + \
    && find /usr/local/lib/gpt-webai-slot-pool/contracts -type f -exec chmod 0644 {} + \
    && ln -sf /usr/local/lib/gpt-webai-slot-pool/provider/chatgpt-playwright/cli.mjs /usr/local/bin/gpt-webai-provider

ENV CHROME_BINARY_PATH=/usr/bin/chromium
ENV CHROME_NO_SANDBOX=1
ENV DISPLAY=:99

ENTRYPOINT ["dumb-init", "--", "gpt-webai-slot-entrypoint"]
