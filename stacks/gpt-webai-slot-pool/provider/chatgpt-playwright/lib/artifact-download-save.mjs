import { copyFile, mkdir, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import { sanitizeFilename, sha256Text } from './common.mjs';

function rawTextEvidence(value) {
  const text = String(value || '');
  return {
    length: text.length,
    sha256: text ? sha256Text(text) : '',
  };
}

export function sanitizedCandidateSnapshot(snapshot) {
  const box = snapshot?.boundingBox || {};
  const className = rawTextEvidence(snapshot?.class);
  const visibleText = rawTextEvidence(snapshot?.visibleText);
  const accessibleName = rawTextEvidence(snapshot?.accessibleName);
  return {
    candidateIndex: snapshot?.candidateIndex,
    role: snapshot?.role || '',
    tag: snapshot?.tag || '',
    classNameLength: className.length,
    classNameSha256: className.sha256,
    visibleTextLength: visibleText.length,
    visibleTextSha256: visibleText.sha256,
    accessibleNameLength: accessibleName.length,
    accessibleNameSha256: accessibleName.sha256,
    boundingBox: {
      x: Number(box.x || 0),
      y: Number(box.y || 0),
      width: Number(box.width || 0),
      height: Number(box.height || 0),
    },
  };
}

function maybeRedact(value, replacement) {
  const text = String(value || '');
  if (text.length < 3) return null;
  return [text, replacement];
}

export function sanitizeArtifactWarningMessage(error, snapshot, visibleFilename) {
  let message = error instanceof Error ? error.message : String(error);
  message = message.replace(/https?:\/\/[^\s'")]+/gi, '[redacted-url]');

  const rawVisibleText = String(snapshot?.visibleText || '');
  const rawAccessibleName = String(snapshot?.accessibleName || '');
  const allowedFilename = String(visibleFilename || '');
  const redactions = [
    maybeRedact(snapshot?.hrefDebug, '[redacted-href]'),
    maybeRedact(snapshot?.turnText, '[redacted-turn-text]'),
    maybeRedact(snapshot?.assistantTurnText, '[redacted-assistant-turn-text]'),
    rawVisibleText === allowedFilename ? null : maybeRedact(rawVisibleText, '[redacted-visible-text]'),
    rawAccessibleName === allowedFilename ? null : maybeRedact(rawAccessibleName, '[redacted-accessible-name]'),
  ].filter(Boolean);

  for (const [raw, replacement] of redactions) {
    message = message.split(raw).join(replacement);
  }

  return message.slice(0, 1000);
}

function isEnoentDownloadSaveError(error) {
  const message = error instanceof Error ? error.message : String(error);
  return /ENOENT|no such file or directory/i.test(message);
}

function positiveIntegerEnv(name, fallback) {
  const value = Number.parseInt(process.env[name] || '', 10);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function attachDownloadSaveFailureMetadata(error, { phase, saveAttempts, suggestedFilename }) {
  if (error instanceof Error) {
    error.phase = phase;
    error.saveAttempts = saveAttempts;
    error.suggestedFilename = suggestedFilename;
  }
  return error;
}

async function existingFile(pathname) {
  try {
    const info = await stat(pathname);
    return info.isFile() && info.size > 0 ? info : null;
  } catch {
    return null;
  }
}

function browserDownloadDirs(directDownloadDir) {
  return Array.from(new Set([
    directDownloadDir,
    ...(process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR || '').split(path.delimiter),
    '/home/node/Downloads',
    '/home/pwuser/Downloads',
  ].map(value => String(value || '').trim()).filter(Boolean)));
}

function filenameStems(filename) {
  if (/\.tar\.gz$/i.test(filename)) return [filename.slice(0, -7), '.tar.gz'];
  const ext = path.extname(filename);
  return [filename.slice(0, filename.length - ext.length), ext];
}

function possibleBrowserDownloadNames(suggestedFilename) {
  const base = path.basename(String(suggestedFilename || ''));
  if (!base || base === '.' || base === '..') return [];
  const sanitized = sanitizeFilename(base, 'download.bin');
  const names = new Set([base, sanitized]);
  for (const filename of [base, sanitized]) {
    const [stem, ext] = filenameStems(filename);
    if (!stem || !ext) continue;
    for (let index = 1; index <= 8; index += 1) {
      names.add(`${stem} (${index})${ext}`);
    }
  }
  return Array.from(names);
}

async function existingBrowserDownloadCandidates(directories, names) {
  const candidates = [];
  for (const directory of directories) {
    for (const name of names) {
      const candidatePath = path.join(directory, name);
      const info = await existingFile(candidatePath);
      if (info) candidates.push({ path: candidatePath, mtimeMs: info.mtimeMs });
    }
    try {
      const entries = await readdir(directory, { withFileTypes: true });
      for (const entry of entries) {
        if (!entry.isFile() || !names.includes(entry.name)) continue;
        const candidatePath = path.join(directory, entry.name);
        const info = await existingFile(candidatePath);
        if (info) candidates.push({ path: candidatePath, mtimeMs: info.mtimeMs });
      }
    } catch {
      // Missing browser download directories are expected in fake/unit runs.
    }
  }
  return candidates.sort((left, right) => right.mtimeMs - left.mtimeMs);
}

async function copyDirectBrowserDownload({ containerPath, directDownloadDir, suggestedFilename }) {
  if (!suggestedFilename) return false;
  const directories = browserDownloadDirs(directDownloadDir);
  const names = possibleBrowserDownloadNames(suggestedFilename);
  if (directories.length === 0 || names.length === 0) return false;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const [candidate] = await existingBrowserDownloadCandidates(directories, names);
    if (candidate) {
      await mkdir(path.dirname(containerPath), { recursive: true });
      if (candidate.path !== containerPath) await copyFile(candidate.path, containerPath);
      return true;
    }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  return false;
}

export async function saveDownloadByClickingCandidate({ page, item, containerPath, timeout, directDownloadDir = '' }) {
  const maxAttempts = positiveIntegerEnv('GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS', 5);
  const retryDelayMs = positiveIntegerEnv('GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_DELAY_MS', 1000);
  const maxRetryDelayMs = positiveIntegerEnv('GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_MAX_DELAY_MS', 5000);
  let phase = 'download.waitForEvent';
  let saveAttempts = 0;
  let suggestedFilename = '';
  let lastError = null;

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    let downloadPromise = null;
    try {
      downloadPromise = page.waitForEvent('download', { timeout });
      phase = 'download.click';
      await item.click({ timeout: 10_000 });
      phase = 'download.waitForEvent';
      const download = await downloadPromise;
      suggestedFilename = download.suggestedFilename();

      phase = 'download.saveAs';
      await mkdir(path.dirname(containerPath), { recursive: true });
      saveAttempts += 1;
      await download.saveAs(containerPath);
      return { suggestedFilename, saveAttempts };
    } catch (error) {
      if (downloadPromise) downloadPromise.catch(() => null);
      if (phase === 'download.saveAs' && isEnoentDownloadSaveError(error)) {
        if (await copyDirectBrowserDownload({ containerPath, directDownloadDir, suggestedFilename })) {
          return { suggestedFilename, saveAttempts, recoveredFrom: 'browser.downloadPath' };
        }
      }
      lastError = error;
      const retryable = phase === 'download.saveAs' && isEnoentDownloadSaveError(error) && attempt < maxAttempts;
      if (!retryable) break;
      const delayMs = Math.min(maxRetryDelayMs, retryDelayMs * (2 ** (attempt - 1)));
      await new Promise(resolve => setTimeout(resolve, delayMs));
    }
  }

  throw attachDownloadSaveFailureMetadata(lastError || new Error(`download.saveAs failed for ${containerPath}`), {
    phase,
    saveAttempts,
    suggestedFilename,
  });
}
