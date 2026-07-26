import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

import { hasProviderLimitDiagnostics } from '../provider/chatgpt-playwright/lib/provider-limit.mjs';

const artifactDir = process.argv[2] || '';
if (!artifactDir) {
  fail('missing artifact dir');
}

const diagnosticsDir = path.join(artifactDir, 'diagnostics');
const matches = [];
let scanned = 0;
const terminalLabels = new Set(['poll-terminal-before-artifacts']);

for (const name of domFiles(diagnosticsDir)) {
  const file = path.join(diagnosticsDir, name);
  scanned += 1;
  const value = JSON.parse(readFileSync(file, 'utf8'));
  if (terminalLabels.has(value.label || '') && hasProviderLimitDiagnostics([value])) {
    matches.push({
      file: path.relative(artifactDir, file),
      label: value.label || '',
      reason: 'provider.limit',
    });
  }
}

if (matches.length > 0) {
  console.log(JSON.stringify({
    ok: false,
    reason: 'provider.limit',
    filesScanned: scanned,
    matches,
  }, null, 2));
  process.exit(1);
}

console.log(JSON.stringify({
  ok: true,
  filesScanned: scanned,
}, null, 2));

function domFiles(dir) {
  try {
    return readdirSync(dir)
      .filter(name => name.endsWith('.dom.json'))
      .sort();
  } catch {
    return [];
  }
}

function fail(message) {
  console.log(JSON.stringify({ ok: false, reason: message }, null, 2));
  process.exit(2);
}
