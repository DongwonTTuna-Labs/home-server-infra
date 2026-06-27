#!/usr/bin/env bash
set -euo pipefail

name="${1:?usage: mcp-suite-stdio <lsp|codegraph|agbrowse> [workspace]}"
workspace="${2:-${PWD:-${MCP_WORKSPACE:-/home/dongwonttuna/Documents/Programming/home-server-infra}}}"
allowed_root="${MCP_ALLOWED_WORKSPACE_ROOT:-/home/dongwonttuna/Documents/Programming}"
export LSP_TOOLS_MCP_PROJECT_CONFIG="${LSP_TOOLS_MCP_PROJECT_CONFIG:-.opencode/lsp.json:.omo/lsp.json:.omo/lsp-client.json}"
allowed_root="$(realpath -m "$allowed_root")"
workspace="$(realpath -m "$workspace")"
case "$workspace" in
  "$allowed_root"|"$allowed_root"/*) ;;
  *)
    printf 'workspace outside allowed root: %s (allowed root: %s)\n' "$workspace" "$allowed_root" >&2
    exit 65
    ;;
esac
cd "$workspace"

case "$name" in
  lsp)
    exec node /opt/mcp-suite/node_modules/oh-my-openagent/packages/lsp-daemon/dist/cli.js mcp
    ;;
  codegraph)
    exec /opt/mcp-suite/node_modules/@colbymchenry/codegraph-linux-x64/bin/codegraph serve --mcp
    ;;
  agbrowse)
    exec node /opt/mcp-suite/node_modules/agbrowse/bin/agbrowse.mjs web-ai mcp-server
    ;;
  *)
    printf 'unknown MCP server: %s\n' "$name" >&2
    exit 64
    ;;
esac
