#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const [caseDir, widthText] = process.argv.slice(2);
const width = Number.parseInt(widthText ?? "", 10);

if (!caseDir || !Number.isInteger(width) || width < 1) {
  throw new Error("usage: live-smoke-slot-summary.mjs CASE_DIR WIDTH");
}

function workerIndex(name) {
  const match = /^worker-(\d+)$/.exec(name);
  return match ? Number.parseInt(match[1], 10) : 0;
}

function readJsonIfPresent(file) {
  if (!fs.existsSync(file) || fs.statSync(file).size === 0) {
    return {};
  }
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function readSummary(file) {
  if (!fs.existsSync(file)) {
    return {};
  }
  const values = {};
  for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    const index = line.indexOf("=");
    if (index === -1) {
      continue;
    }
    values[line.slice(0, index)] = line.slice(index + 1);
  }
  return values;
}

function unique(values) {
  return Array.from(new Set(values));
}

const workerDirs = fs
  .readdirSync(caseDir, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && /^worker-\d+$/.test(entry.name))
  .map((entry) => entry.name)
  .sort((left, right) => workerIndex(left) - workerIndex(right));

const workers = workerDirs.map((worker) => {
  const workerDir = path.join(caseDir, worker);
  const run = readJsonIfPresent(path.join(workerDir, "run.out"));
  const summary = readSummary(path.join(workerDir, "session-summary.txt"));
  return {
    worker,
    sessionId: run.sessionId ?? summary.sessionId,
    conversationUrl: run.conversationUrl ?? summary.conversationUrl,
    slotId: run.slotId ?? summary.slotId,
    accountGroup: run.accountGroup ?? summary.accountGroup,
    runId: run.runId ?? summary.runId,
    ok: run.ok,
    status: run.status,
    lockAcquired: run.lockAcquired,
    lockReleased: run.lockReleased,
    runtimeStopped: run.runtimeStopped,
  };
});

const sessions = workers.map((worker) => worker.sessionId).filter(Boolean);
const slots = workers.map((worker) => worker.slotId).filter(Boolean);
const duplicateSlots = unique(slots.filter((slot) => slots.indexOf(slot) !== slots.lastIndexOf(slot))).sort();

if (workers.length !== width) {
  throw new Error(`worker count ${workers.length} did not reach width ${width}`);
}
if (sessions.length !== width || unique(sessions).length !== width) {
  throw new Error(`sessions were not unique at width ${width}: ${JSON.stringify(sessions)}`);
}
if (slots.length !== width || unique(slots).length !== width) {
  throw new Error(`slots were not unique at width ${width}: ${JSON.stringify(slots)}`);
}

const slotSummary = {
  schema: "gpt-webai.live-smoke.slot-lease-summary.v1",
  requestedWidth: width,
  workerCount: workers.length,
  uniqueSessions: unique(sessions).length,
  uniqueSlots: unique(slots).length,
  requestedWidthReached: workers.length === width && unique(slots).length === width,
  overlappingSameSlotLeases: duplicateSlots.length > 0,
  duplicateSlots,
  workers,
};

fs.writeFileSync(
  path.join(caseDir, "slot-lease-summary.json"),
  `${JSON.stringify(slotSummary, null, 2)}\n`,
);
fs.writeFileSync(
  path.join(caseDir, "session-uniqueness.json"),
  `${JSON.stringify(
    {
      sessions,
      slots,
      uniqueSessions: unique(sessions).length,
      uniqueSlots: unique(slots).length,
    },
    null,
    2,
  )}\n`,
);
