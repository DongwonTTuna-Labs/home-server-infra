FROM node:24-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    git \
    libasound2 \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libatspi2.0-0 \
    libcairo2 \
    libcups2 \
    libdbus-1-3 \
    libgbm1 \
    libglib2.0-0 \
    libgtk-3-0 \
    libnspr4 \
    libnss3 \
    libpango-1.0-0 \
    libx11-6 \
    libxcb1 \
    libxcomposite1 \
    libxdamage1 \
    libxext6 \
    libxfixes3 \
    libxkbcommon0 \
    libxrandr2 \
    libxshmfence1 \
    tini \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/mcp-suite
RUN npm init -y \
    && npm install --omit=dev \
      @colbymchenry/codegraph-linux-x64@latest \
      @modelcontextprotocol/sdk@latest \
      agbrowse@latest \
      mcp-proxy@latest \
      oh-my-openagent@latest

COPY scripts/start.sh /usr/local/bin/mcp-suite-start
COPY scripts/stdio.sh /usr/local/bin/mcp-suite-stdio
COPY scripts/healthcheck.sh /usr/local/bin/mcp-suite-healthcheck
COPY scripts/smoke.sh /usr/local/bin/mcp-suite-smoke
RUN chmod 0755 /usr/local/bin/mcp-suite-start \
    /usr/local/bin/mcp-suite-stdio \
    /usr/local/bin/mcp-suite-healthcheck \
    /usr/local/bin/mcp-suite-smoke \
    && mkdir -p /var/log/mcp-suite \
    && chown -R 1000:1000 /opt/mcp-suite /var/log/mcp-suite

ENV NODE_PATH=/opt/mcp-suite/node_modules
ENV PATH=/opt/mcp-suite/node_modules/.bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
ENV HOME=/home/dongwonttuna
ENV MCP_WORKSPACE=/home/dongwonttuna/Documents/Programming/home-server-infra

USER 1000:1000
WORKDIR /home/dongwonttuna/Documents/Programming/home-server-infra
EXPOSE 8301 8302 8303
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["mcp-suite-start"]
