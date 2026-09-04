import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeIntelligenceLabel,
  parsePillLabel,
  renderedTurnMatchesPrompt,
} from "../../src/daemon/selectors.js";
test("Intelligence labels normalize case, whitespace, and version badges on any line", () => {
  assert.equal(normalizeIntelligenceLabel(" Pro "), "pro");
  assert.equal(normalizeIntelligenceLabel("Instant\n5.5"), "instant");
  assert.equal(normalizeIntelligenceLabel("  Extra\t High  \nBadge"), "extra high badge");
  // 2026-09 GPT-6 UI: 버전이 첫 줄, power가 둘째 줄.
  assert.equal(normalizeIntelligenceLabel("6\nPro"), "pro");
  assert.equal(normalizeIntelligenceLabel("6.1\nExtra High"), "extra high");
});
test("pill labels split into version and power for evidence and version checks", () => {
  assert.deepEqual(parsePillLabel("6\nPro"), { version: "6", power: "Pro", display: "6 Pro" });
  assert.deepEqual(parsePillLabel("Pro"), { version: null, power: "Pro", display: "Pro" });
  assert.deepEqual(parsePillLabel("Instant\n5.5"), { version: "5.5", power: "Instant", display: "Instant 5.5" });
});
test("rendered prompts ignore fences and leading attachment labels without losing Unicode", () => {
  const prompt = "파이썬 실행\r\n```python\r\n  print(\"안녕 🧪\")  \r\n```\r\n완료";
  const prefix = "bundle.tar.gz\nFile\n파이썬 실행\n";
  assert.equal(renderedTurnMatchesPrompt(`${prefix}python\nprint(\"안녕 🧪\")\n완료`, prompt), true);
  assert.equal(renderedTurnMatchesPrompt(`${prefix}javascript\nprint(\"안녕 🧪\")\n완료`, prompt), false);
  assert.equal(renderedTurnMatchesPrompt(`${prefix}python\nprint(\"다름 🧪\")\n완료`, prompt), false);
});
