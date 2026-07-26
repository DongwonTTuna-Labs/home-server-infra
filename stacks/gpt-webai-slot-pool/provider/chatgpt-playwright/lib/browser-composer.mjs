import path from 'node:path';
import process from 'node:process';

import {
  DEFAULT_ATTACHMENT_TIMEOUT_MS,
  validConversationUrl,
} from './common.mjs';

export async function visibleLocator(page, selectors) {
  for (const selector of selectors) {
    const locator = page.locator(selector).first();
    if (await locator.count().catch(() => 0)) {
      if (await locator.isVisible().catch(() => false)) return locator;
    }
  }
  return null;
}

export async function fillPrompt(page, prompt) {
  const composer = await promptComposer(page);
  if (!composer) throw new Error('composer not ready');
  await composer.click({ timeout: 10_000 });
  try {
    await composer.fill(prompt, { timeout: 10_000 });
  } catch {
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+A' : 'Control+A');
    await page.keyboard.insertText(prompt);
  }
}

export async function readPromptComposer(page) {
  const composer = await promptComposer(page);
  if (!composer) throw new Error('composer not ready');
  return composer.evaluate(node => (
    'value' in node ? String(node.value || '') : String(node.innerText || node.textContent || '')
  ));
}

async function promptComposer(page) {
  return visibleLocator(page, [
    '#prompt-textarea',
    'textarea[placeholder*="Message" i]',
    '[contenteditable="true"][role="textbox"]',
    '.ProseMirror[contenteditable="true"]',
    '[contenteditable="true"]',
  ]);
}

export async function setFiles(page, files) {
  if (files.length === 0) return;
  const input = page.locator('input[type="file"]').first();
  if (await input.count().catch(() => 0)) {
    await input.setInputFiles(files, { timeout: 30_000 });
    return;
  }
  const upload = await visibleLocator(page, [
    'button[aria-label*="attach" i]',
    'button[aria-label*="upload" i]',
    'button[aria-label*="file" i]',
    'button[data-testid*="upload" i]',
  ]);
  if (!upload) throw new Error('upload control not ready');
  const chooserPromise = page.waitForEvent('filechooser', { timeout: 10_000 });
  await upload.click({ timeout: 10_000 });
  const chooser = await chooserPromise;
  await chooser.setFiles(files);
}

export async function waitForAttachmentEvidence(page, files) {
  if (files.length === 0) return { ok: true, expected: [], observed: [] };
  const expected = files.map(file => path.basename(file));
  const timeout = Number.parseInt(process.env.GPT_WEBAI_ATTACHMENT_READY_TIMEOUT_MS || '', 10) || DEFAULT_ATTACHMENT_TIMEOUT_MS;
  const deadline = Date.now() + timeout;
  let latest = [];
  while (Date.now() < deadline) {
    latest = await page.evaluate(names => {
      const visible = node => {
        if (!node || typeof node.getBoundingClientRect !== 'function') return false;
        const rect = node.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return false;
        const style = window.getComputedStyle?.(node);
        return !style || (style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0');
      };
      const nodes = Array.from(document.querySelectorAll('[data-testid*="attachment" i], [class*="attachment" i], [class*="file" i], button, [role="button"], span, div'))
        .filter(visible)
        .map(node => ({
          text: (node.innerText || node.textContent || '').trim(),
          label: node.getAttribute('aria-label') || '',
          title: node.getAttribute('title') || '',
        }));
      return names.map(name => {
        // ChatGPT renames duplicate uploads ("name(1).ext"), so match on the
        // extension-less stem rather than the exact filename.
        const stem = name.replace(/\.[^.]+$/, '').toLowerCase();
        const hit = nodes.find(node => `${node.text}\n${node.label}\n${node.title}`.toLowerCase().includes(stem));
        return hit ? { expected: name, visible: true, text: hit.text || hit.label || hit.title } : { expected: name, visible: false };
      });
    }, expected).catch(() => []);
    if (latest.length === expected.length && latest.every(item => item.visible)) {
      // Names visible is not upload completion. ChatGPT disables the send
      // button while uploads are in flight, so button enablement is the
      // authoritative completion signal; require it stable across two checks.
      const sendEnabled = async () => {
        const send = await visibleLocator(page, [
          'button[data-testid*="send" i]',
          'button[aria-label*="send" i]',
        ]);
        if (!send) return false;
        return send.isEnabled().catch(() => false);
      };
      if (await sendEnabled()) {
        await page.waitForTimeout(700);
        if (await sendEnabled()) {
          return { ok: true, expected, observed: latest };
        }
      }
    }
    await page.waitForTimeout(500);
  }
  return {
    ok: false,
    expected,
    observed: latest,
    missing: latest.filter(item => !item.visible).map(item => item.expected),
  };
}

export async function clickSend(page) {
  const timeout = Number.parseInt(process.env.GPT_WEBAI_SEND_READY_TIMEOUT_MS || '', 10) || DEFAULT_ATTACHMENT_TIMEOUT_MS;
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const send = await visibleLocator(page, [
      'button[data-testid*="send" i]',
      'button[aria-label*="send" i]',
    ]);
    if (send && await send.isEnabled().catch(() => true)) {
      await send.click({ timeout: 10_000 });
      return;
    }
    await page.waitForTimeout(500);
  }
  throw new Error('send button not ready');
}

export async function waitForConversationUrl(page, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const url = page.url();
    if (validConversationUrl(url)) return url;
    await page.waitForTimeout(500);
  }
  return page.url();
}
