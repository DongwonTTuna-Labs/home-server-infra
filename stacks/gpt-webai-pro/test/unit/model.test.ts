import assert from "node:assert/strict";
import test from "node:test";

import { normalizeIntelligenceLabel } from "../../src/daemon/selectors.js";

test("Intelligence labels normalize case, whitespace, and first-line badges", () => {
  assert.equal(normalizeIntelligenceLabel(" Pro "), "pro");
  assert.equal(normalizeIntelligenceLabel("Instant\n5.5"), "instant");
  assert.equal(normalizeIntelligenceLabel("  Extra\t High  \nBadge"), "extra high");
});
