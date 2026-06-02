FROM ubuntu:24.04

ARG RUNNER_VERSION=2.334.0
ARG NODE_MAJOR=24
ARG RUST_VERSION=1.95.0
ARG CODEX_CLI_VERSION=0.135.0
ARG GH_VERSION=2.93.0

ENV DEBIAN_FRONTEND=noninteractive
ENV BUN_INSTALL=/usr/local/bun
ENV CARGO_HOME=/usr/local/cargo
ENV RUNNER_HOME=/home/runner/actions-runner
ENV RUNNER_WORKDIR=/home/runner/_work
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
        gzip \
        jq \
        lsb-release \
        libssl-dev \
        openssh-client \
        pkg-config \
        python3 \
        python3-pip \
        python3-yaml \
        sudo \
        tar \
        tini \
        unzip \
        util-linux \
        xz-utils \
        zip \
        zstd \
    && rm -rf /var/lib/apt/lists/*

RUN command -v lsb_release \
    && lsb_release -a

RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) gh_arch="amd64" ;; \
      arm64) gh_arch="arm64" ;; \
      *) echo "Unsupported GitHub CLI architecture: $arch" >&2; exit 1 ;; \
    esac; \
    tarball="gh_${GH_VERSION}_linux_${gh_arch}.tar.gz"; \
    checksums="gh_${GH_VERSION}_checksums.txt"; \
    release_url="https://github.com/cli/cli/releases/download/v${GH_VERSION}"; \
    curl -fsSL "$release_url/$tarball" -o "/tmp/$tarball"; \
    curl -fsSL "$release_url/$checksums" -o "/tmp/$checksums"; \
    cd /tmp; \
    grep "  $tarball$" "$checksums" | sha256sum -c -; \
    tar -xzf "/tmp/$tarball" -C /tmp; \
    install -m 0755 "/tmp/gh_${GH_VERSION}_linux_${gh_arch}/bin/gh" /usr/local/bin/gh; \
    rm -rf "/tmp/$tarball" "/tmp/$checksums" "/tmp/gh_${GH_VERSION}_linux_${gh_arch}"; \
    gh --version

RUN useradd --create-home --shell /bin/bash --uid 1001 runner \
    && mkdir -p \
        "${BUN_INSTALL}" \
        "${CARGO_HOME}" \
        "${RUNNER_HOME}" \
        "${RUNNER_WORKDIR}" \
        "${NPM_CONFIG_PREFIX}" \
        "${RUSTUP_HOME}" \
        /opt/codex-runner

# openai/codex-action starts a responses-api-proxy and root-owns its server-info
# file via `sudo chmod 444` / `sudo chown root` (unconditional, no opt-out). Grant
# the runner passwordless sudo scoped to exactly those two commands so the native
# proxy path works; no broader sudo is permitted.
RUN printf '%s\n' 'runner ALL=(root) NOPASSWD: /usr/bin/chmod, /usr/bin/chown' > /etc/sudoers.d/runner-codex-action \
    && chmod 0440 /etc/sudoers.d/runner-codex-action \
    && visudo -cf /etc/sudoers.d/runner-codex-action

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
    && npm cache clean --force

RUN npm install -g "@openai/codex@${CODEX_CLI_VERSION}" \
    && command -v codex \
    && codex --version \
    && npm cache clean --force

RUN curl -fsSL https://bun.sh/install \
    | bash \
    && ln -sf "${BUN_INSTALL}/bin/bun" /usr/local/bin/bun \
    && ln -sf "${BUN_INSTALL}/bin/bunx" /usr/local/bin/bunx

RUN curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}" \
    && rustup component add clippy rustfmt

COPY scripts/entrypoint.sh /opt/codex-runner/entrypoint.sh

RUN chmod 0755 \
        /opt/codex-runner/entrypoint.sh \
    && chown -R runner:runner \
        "${BUN_INSTALL}" \
        "${CARGO_HOME}" \
        /home/runner \
        /opt/codex-runner \
        "${RUSTUP_HOME}"

ENV HOME=/home/runner

USER runner

ENTRYPOINT ["/usr/bin/tini", "--", "/opt/codex-runner/entrypoint.sh"]
