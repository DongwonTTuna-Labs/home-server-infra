FROM ubuntu:24.04 AS auth-guard-builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        gcc \
        libc6-dev \
    && rm -rf /var/lib/apt/lists/*

COPY src/codex-deny-auth.c /src/codex-deny-auth.c

RUN gcc -shared -fPIC -O2 -Wall -Wextra \
        -o /libcodex-deny-auth.so \
        /src/codex-deny-auth.c \
        -ldl

FROM ubuntu:24.04

ARG RUNNER_VERSION=2.334.0
ARG NODE_VERSION=20.19.5
ARG CODEX_CLI_VERSION=latest

ENV DEBIAN_FRONTEND=noninteractive
ENV CODEX_CLI_AUTO_UPDATE=1
ENV CODEX_CLI_VERSION=${CODEX_CLI_VERSION}
ENV RUNNER_HOME=/home/runner/actions-runner
ENV RUNNER_WORKDIR=/home/runner/_work
ENV CODEX_HOME=/home/runner/.codex
ENV RUNNER_TOOL_CACHE=/home/runner/_work/_tool
ENV RUNNER_TEMP=/home/runner/_work/_temp
ENV NPM_CONFIG_PREFIX=/home/runner/.npm-global
ENV PATH=/home/runner/.npm-global/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
        git \
        gh \
        gzip \
        openssh-client \
        python3 \
        sudo \
        tar \
        tini \
        unzip \
        util-linux \
        xz-utils \
        zip \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash --uid 1001 runner \
    && mkdir -p "${RUNNER_HOME}" "${RUNNER_WORKDIR}" "${CODEX_HOME}" "${NPM_CONFIG_PREFIX}" /opt/codex-runner/hooks /var/lib/codex-runner/locks

WORKDIR ${RUNNER_HOME}

RUN curl -fsSL -o actions-runner.tar.gz \
        "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
    && tar -xzf actions-runner.tar.gz \
    && rm actions-runner.tar.gz \
    && ./bin/installdependencies.sh

RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
    | tar -xJ -C /usr/local --strip-components=1 \
    && npm install -g "@openai/codex@${CODEX_CLI_VERSION}" \
    && npm cache clean --force

COPY hooks/job-started.sh /opt/codex-runner/hooks/job-started.sh
COPY hooks/job-completed.sh /opt/codex-runner/hooks/job-completed.sh
COPY scripts/entrypoint.sh /opt/codex-runner/entrypoint.sh
COPY scripts/update-codex-cli.sh /opt/codex-runner/update-codex-cli.sh
COPY --from=auth-guard-builder /libcodex-deny-auth.so /opt/codex-runner/libcodex-deny-auth.so

RUN chmod 0755 \
        /opt/codex-runner/entrypoint.sh \
        /opt/codex-runner/update-codex-cli.sh \
        /opt/codex-runner/hooks/job-started.sh \
        /opt/codex-runner/hooks/job-completed.sh \
        /opt/codex-runner/libcodex-deny-auth.so \
    && chown -R runner:runner /home/runner /opt/codex-runner /var/lib/codex-runner

USER runner

ENTRYPOINT ["/usr/bin/tini", "--", "/opt/codex-runner/entrypoint.sh"]
