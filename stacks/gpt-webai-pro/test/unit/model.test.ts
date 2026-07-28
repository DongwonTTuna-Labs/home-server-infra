import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeIntelligenceLabel,
  renderedTurnMatchesPrompt,
} from "../../src/daemon/selectors.js";
test("Intelligence labels normalize case, whitespace, and first-line badges", () => {
  assert.equal(normalizeIntelligenceLabel(" Pro "), "pro");
  assert.equal(normalizeIntelligenceLabel("Instant\n5.5"), "instant");
  assert.equal(normalizeIntelligenceLabel("  Extra\t High  \nBadge"), "extra high");
});
test("rendered prompts ignore fences and leading attachment labels without losing Unicode", () => {
  const prompt = "파이썬 실행\r\n```python\r\n  print(\"안녕 🧪\")  \r\n```\r\n완료";
  const prefix = "bundle.tar.gz\nFile\n파이썬 실행\n";
  assert.equal(renderedTurnMatchesPrompt(`${prefix}python\nprint(\"안녕 🧪\")\n완료`, prompt), true);
  assert.equal(renderedTurnMatchesPrompt(`${prefix}javascript\nprint(\"안녕 🧪\")\n완료`, prompt), false);
  assert.equal(renderedTurnMatchesPrompt(`${prefix}python\nprint(\"다름 🧪\")\n완료`, prompt), false);
});
