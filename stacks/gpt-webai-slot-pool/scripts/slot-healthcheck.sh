#!/usr/bin/env bash
set -euo pipefail

: "${BROWSER_AGENT_HOME:?BROWSER_AGENT_HOME is required}"
: "${CDP_PORT:?CDP_PORT is required}"

test -d "$BROWSER_AGENT_HOME"
curl -fsS "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null
