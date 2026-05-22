FROM rust:1-bookworm

ARG NODE_MAJOR=24

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        python3 \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

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
    grep "  $tarball$" /tmp/SHASUMS256.txt | sha256sum -c -; \
    tar -xJf "/tmp/$tarball" -C /usr/local --strip-components=1; \
    rm -f "/tmp/$tarball" /tmp/SHASUMS256.txt; \
    node --version; \
    npm --version; \
    rustc --version; \
    cargo --version
