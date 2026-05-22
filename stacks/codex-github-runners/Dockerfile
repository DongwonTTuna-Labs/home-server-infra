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
ARG NODE_MAJOR=24
ARG RUST_VERSION=1.95.0
ARG CODEX_CLI_VERSION=latest

ENV DEBIAN_FRONTEND=noninteractive
ENV BUN_INSTALL=/usr/local/bun
ENV CODEX_CLI_AUTO_UPDATE=1
ENV CODEX_CLI_VERSION=${CODEX_CLI_VERSION}
ENV CARGO_HOME=/usr/local/cargo
ENV RUNNER_HOME=/home/runner/actions-runner
ENV RUNNER_WORKDIR=/home/runner/_work
ENV CODEX_HOME=/home/runner/.codex
ENV RUSTUP_HOME=/usr/local/rustup
ENV RUNNER_TOOL_CACHE=/home/runner/_work/_tool
ENV RUNNER_TEMP=/home/runner/_work/_temp
ENV NPM_CONFIG_PREFIX=/home/runner/.npm-global
ENV PATH=/home/runner/.npm-global/bin:/usr/local/cargo/bin:/usr/local/bun/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        build-essential \
        ca-certificates \
        curl \
        git \
        gh \
        gzip \
        jq \
        libssl-dev \
        openssh-client \
        pkg-config \
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
    && mkdir -p \
        "${BUN_INSTALL}" \
        "${CARGO_HOME}" \
        "${RUNNER_HOME}" \
        "${RUNNER_WORKDIR}" \
        "${CODEX_HOME}" \
        "${NPM_CONFIG_PREFIX}" \
        "${RUSTUP_HOME}" \
        /opt/codex-runner/hooks \
        /var/lib/codex-runner/locks

WORKDIR ${RUNNER_HOME}

RUN curl -fsSL -o actions-runner.tar.gz \
        "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
    && tar -xzf actions-runner.tar.gz \
    && rm actions-runner.tar.gz \
    && ./bin/installdependencies.sh

RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) node_arch="x64" ;; \
      arm64) node_arch="arm64" ;; \
      *) echo "Unsupported Node architecture: $arch" >&2; exit 1 ;; \
    esac; \
    release_url="https://nodejs.org/dist/latest-v${NODE_MAJOR}.x"; \
    tarball="$(curl -fsSL "$release_url/SHASUMS256.txt" \
      | awk -v arch="$node_arch" '$2 ~ "^node-v[0-9]+[.][0-9]+[.][0-9]+-linux-" arch "[.]tar[.]xz$" { print $2; exit }')"; \
    test -n "$tarball"; \
    curl -fsSL "$release_url/$tarball" -o "/tmp/$tarball"; \
    curl -fsSL "$release_url/SHASUMS256.txt" -o /tmp/SHASUMS256.txt; \
    cd /tmp; \
    grep "  $tarball$" SHASUMS256.txt | sha256sum -c -; \
    tar -xJf "/tmp/$tarball" -C /usr/local --strip-components=1; \
    rm -f "/tmp/$tarball" /tmp/SHASUMS256.txt \
    && npm install -g "@openai/codex@${CODEX_CLI_VERSION}" \
    && npm cache clean --force

RUN curl -fsSL https://bun.sh/install \
    | bash \
    && ln -sf "${BUN_INSTALL}/bin/bun" /usr/local/bin/bun \
    && ln -sf "${BUN_INSTALL}/bin/bunx" /usr/local/bin/bunx

RUN curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}" \
    && rustup component add clippy rustfmt

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
    && chown -R runner:runner \
        "${BUN_INSTALL}" \
        "${CARGO_HOME}" \
        /home/runner \
        /opt/codex-runner \
        "${RUSTUP_HOME}" \
        /var/lib/codex-runner

USER runner

ENTRYPOINT ["/usr/bin/tini", "--", "/opt/codex-runner/entrypoint.sh"]
