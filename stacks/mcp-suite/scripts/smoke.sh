#!/usr/bin/env bash
set -euo pipefail

mcp-suite-healthcheck
cd /opt/mcp-suite
node --input-type=module <<'NODE'
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const servers = ["lsp", "codegraph", "agbrowse"];
const workspace = process.env.MCP_WORKSPACE ?? "/home/dongwonttuna/Documents/Programming/home-server-infra";

for (const name of servers) {
  const client = new Client({ name: `mcp-suite-smoke-${name}`, version: "1.0.0" });
  const transport = new StdioClientTransport({
    command: "mcp-suite-stdio",
    args: [name, workspace],
  });
  await client.connect(transport);
  const tools = await client.listTools();
  if (!Array.isArray(tools.tools) || tools.tools.length === 0) {
    throw new Error(`${name} returned no tools`);
  }
  console.log(`${name}: ${tools.tools.length} tools`);
  await client.close();
}
NODE
