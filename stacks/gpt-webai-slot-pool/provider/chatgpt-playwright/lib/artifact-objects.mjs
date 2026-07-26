import { readFile } from 'node:fs/promises';

import { sha256Text } from './common.mjs';

export function canonicalArtifactObject({
  sessionId,
  buttonText,
  turnIndex,
  turnScope,
  clickedElement,
  artifact,
}) {
  return {
    sessionId,
    buttonText,
    buttonTextSha256: buttonText ? sha256Text(buttonText) : '',
    turnScope,
    clickedElement,
    artifact,
    // Legacy shape kept while the Bash supervisor is still being migrated.
    element: {
      ...clickedElement,
      turnIndex,
      turnScope,
    },
    download: artifact.status === 'saved' ? artifact : undefined,
  };
}

function artifactNames(object) {
  const artifact = object?.artifact || {};
  return new Set([
    object?.buttonText,
    artifact.visibleFilename,
    artifact.suggestedFilename,
    artifact.finalFilename,
    artifact.finalFilename ? artifact.finalFilename.replace(/^\d+-/, '') : '',
  ].map(value => String(value || '').trim()).filter(Boolean));
}

function sidecarBaseNames(object) {
  const bases = [];
  for (const name of artifactNames(object)) {
    if (/\.sha256$/i.test(name)) bases.push(name.replace(/\.sha256$/i, ''));
  }
  return bases;
}

export async function annotateSidecarRelationships(artifacts) {
  for (const sidecar of artifacts) {
    const baseNames = sidecarBaseNames(sidecar);
    if (baseNames.length === 0) continue;

    const target = artifacts.find(candidate => (
      candidate !== sidecar
      && sidecarBaseNames(candidate).length === 0
      && baseNames.some(baseName => artifactNames(candidate).has(baseName))
    ));
    const sidecarText = await readFile(sidecar.artifact?.containerPath || '', 'utf8').catch(() => '');
    const declaredSha256 = (sidecarText.match(/\b[0-9a-f]{64}\b/i) || [''])[0].toLowerCase();

    if (!target) {
      sidecar.artifact.integrity = {
        status: 'orphan_sidecar',
        declaredSha256,
      };
      continue;
    }

    const targetSha256 = String(target.artifact?.sha256 || '').toLowerCase();
    const verified = Boolean(declaredSha256 && targetSha256 && declaredSha256 === targetSha256);
    sidecar.artifact.sidecarOf = {
      buttonText: target.buttonText,
      buttonTextSha256: target.buttonTextSha256,
      finalFilename: target.artifact?.finalFilename || '',
      sha256: target.artifact?.sha256 || '',
    };
    sidecar.artifact.integrity = {
      status: verified ? 'verified' : 'mismatch',
      declaredSha256,
      targetSha256,
    };
    target.artifact.integrity = {
      ...(target.artifact.integrity || {}),
      sha256Sidecar: verified ? 'verified' : 'mismatch',
    };
  }
}
