#!/usr/bin/env bash
set -euo pipefail

repo="$HOME/Documents/Programming/home-server-infra"
compose="$repo/stacks/mcp-suite/compose.yaml"
image="home-server/mcp-suite"
candidate="${image}:candidate"
rollback="${image}:rollback"
latest="${image}:latest"
candidate_name="mcp-suite-candidate"
promoted=0
had_rollback=0

cleanup_candidate() {
  docker rm -f "$candidate_name" >/dev/null 2>&1 || true
}

wait_healthy() {
  local name="$1"
  for _ in {1..60}; do
    if docker exec "$name" mcp-suite-healthcheck >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  docker exec "$name" mcp-suite-healthcheck
}

rollback_latest() {
  if [ "$had_rollback" -ne 1 ]; then
    printf 'No rollback image available for %s\n' "$latest" >&2
    return 1
  fi
  docker tag "$rollback" "$latest"
  docker compose -f "$compose" up -d --force-recreate mcp-suite
  wait_healthy mcp-suite
  docker exec mcp-suite mcp-suite-smoke
}

on_exit() {
  local status="$1"
  cleanup_candidate
  if [ "$status" -ne 0 ] && [ "$promoted" -eq 1 ]; then
    rollback_latest || true
  fi
  exit "$status"
}
trap 'on_exit $?' EXIT

if docker image inspect "$latest" >/dev/null 2>&1; then
  docker tag "$latest" "$rollback"
  had_rollback=1
fi

docker build --pull --no-cache -t "$candidate" -f "$repo/stacks/mcp-suite/Dockerfile" "$repo/stacks/mcp-suite"
cleanup_candidate
docker run -d \
  --name "$candidate_name" \
  --restart no \
  --label com.centurylinklabs.watchtower.enable=false \
  -e HOME=/home/dongwonttuna \
  -e MCP_WORKSPACE=/home/dongwonttuna/Documents/Programming/home-server-infra \
  -e MCP_ALLOWED_WORKSPACE_ROOT=/home/dongwonttuna/Documents/Programming \
  -e OMO_DISABLE_POSTHOG=1 \
  -e OMO_SEND_ANONYMOUS_TELEMETRY=0 \
  -e OMO_CODEX_DISABLE_POSTHOG=1 \
  -e OMO_CODEX_SEND_ANONYMOUS_TELEMETRY=0 \
  -e CODEGRAPH_TELEMETRY=0 \
  -e DO_NOT_TRACK=1 \
  -e LSP_TOOLS_MCP_PROJECT_CONFIG=.opencode/lsp.json:.omo/lsp.json:.omo/lsp-client.json \
  -e CHROME_BINARY_PATH=/home/dongwonttuna/.local/chrome-for-testing/current/chrome \
  -e CHROME_NO_SANDBOX=1 \
  -e DISPLAY=:99 \
  -e AGBROWSE_CHROME_FLAGS='--disable-dev-shm-usage --disable-gpu' \
  -v /home/dongwonttuna/Documents/Programming:/home/dongwonttuna/Documents/Programming \
  -v /home/dongwonttuna/.local/chrome-for-testing:/home/dongwonttuna/.local/chrome-for-testing:ro \
  "$candidate"

wait_healthy "$candidate_name"
if [ "${MCP_SUITE_FORCE_CANDIDATE_SMOKE_FAIL:-0}" = "1" ]; then
  printf 'Forced candidate smoke failure before promotion\n' >&2
  exit 86
fi
docker exec "$candidate_name" mcp-suite-smoke

docker tag "$candidate" "$latest"
promoted=1
docker compose -f "$compose" up -d mcp-suite
wait_healthy mcp-suite
if [ "${MCP_SUITE_FORCE_LIVE_SMOKE_FAIL:-0}" = "1" ]; then
  printf 'Forced live smoke failure after promotion\n' >&2
  exit 87
fi
docker exec mcp-suite mcp-suite-smoke
promoted=0
