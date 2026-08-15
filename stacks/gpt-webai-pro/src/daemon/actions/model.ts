import type { Page } from "playwright-core";
import { GwpError } from "../../shared/errors.js";
import type { LabelConfig } from "../../shared/types.js";
import {
  findIntelligenceOption,
  findIntelligencePill,
  normalizeIntelligenceLabel,
  readCurrentModelLabel,
  waitForIntelligenceMenu,
} from "../selectors.js";
export async function ensurePro(page: Page, labels: LabelConfig): Promise<string> {
  const targets = new Set(labels.target.map(normalizeIntelligenceLabel));
  try {
    const pill = await findIntelligencePill(page, labels.intelligence);
    if (!pill) throw new Error("intelligence picker pill is not uniquely visible");
    let current = await pill.innerText().catch(() => "");
    if (targets.has(normalizeIntelligenceLabel(current))) return current.trim();
    await pill.click({ timeout: 10_000 });
    if (!await waitForIntelligenceMenu(page, 10_000)) {
      throw new Error("intelligence radio menu did not become visible");
    }
    const target = await findIntelligenceOption(page, labels.target);
    if (!target) {
      throw new Error(`target intelligence is unavailable: ${labels.target.join(", ")}`);
    }
    await target.click({ timeout: 10_000 });
    await page.waitForTimeout(500);
    if (await target.getAttribute("aria-checked", { timeout: 5_000 }).catch(() => null) !== "true") {
      throw new Error("target intelligence did not become aria-checked");
    }
    current = await readCurrentModelLabel(page, labels.intelligence);
    if (!targets.has(normalizeIntelligenceLabel(current))) {
      throw new Error(`intelligence pill recheck failed; current label is ${JSON.stringify(current)}`);
    }
    return current.trim();
  } catch (error) {
    if (error instanceof GwpError) throw error;
    throw new GwpError("model_unavailable", String(error), { phase: "pre_click", cause: error });
  }
}
