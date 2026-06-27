#!/usr/bin/env bash
set -euo pipefail

export HOME="${HOME:-/home/dongwonttuna}"
export MCP_WORKSPACE="${MCP_WORKSPACE:-/home/dongwonttuna/Documents/Programming/home-server-infra}"
export LSP_TOOLS_MCP_PROJECT_CONFIG="${LSP_TOOLS_MCP_PROJECT_CONFIG:-.opencode/lsp.json:.omo/lsp.json:.omo/lsp-client.json}"
mkdir -p /var/log/mcp-suite

launch_proxy() {
  local name="$1"
  local port="$2"
  local command="$3"
  local stateless_flag="${4:-}"
  if [ -n "$stateless_flag" ]; then
    mcp-proxy --host 0.0.0.0 --port "$port" --server stream --streamEndpoint /mcp "$stateless_flag" --shell -- "$command" \
      >"/var/log/mcp-suite/${name}.log" 2>&1 &
  else
    mcp-proxy --host 0.0.0.0 --port "$port" --server stream --streamEndpoint /mcp --shell -- "$command" \
      >"/var/log/mcp-suite/${name}.log" 2>&1 &
  fi
  printf '%s %s\n' "$name" "$!" >&2
}

launch_proxy lsp 8301 "cd '$MCP_WORKSPACE' && exec node /opt/mcp-suite/node_modules/oh-my-openagent/packages/lsp-daemon/dist/cli.js mcp" --stateless
launch_proxy codegraph 8302 "cd '$MCP_WORKSPACE' && exec /opt/mcp-suite/node_modules/@colbymchenry/codegraph-linux-x64/bin/codegraph serve --mcp"
launch_proxy agbrowse 8303 "cd '$MCP_WORKSPACE' && exec node /opt/mcp-suite/node_modules/agbrowse/bin/agbrowse.mjs web-ai mcp-server"

trap 'jobs -p | xargs -r kill' INT TERM
wait -n
