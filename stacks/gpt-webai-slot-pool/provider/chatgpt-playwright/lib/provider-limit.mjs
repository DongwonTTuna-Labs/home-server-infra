export const PROVIDER_LIMIT_RE = /too many requests|request limit|rate limit|usage limit|temporarily limited|message cap|try again later|you(?:'|’| have)?ve? reached (?:the )?.{0,80}limit/i;

export function hasProviderLimitDiagnostics(diagnostics = []) {
  return diagnostics.some(diagnosticHasProviderLimit);
}

export function applyProviderLimitStatus(payload, diagnostics = payload?.diagnostics || []) {
  if (!hasProviderLimitDiagnostics(diagnostics)) return false;
  payload.status = 'provider_limit';
  payload.reason = 'provider.limit';
  return true;
}

function diagnosticHasProviderLimit(diagnostic = {}) {
  if (Array.isArray(diagnostic.providerLimitSurfaces)) {
    return hasProviderLimitSurface(diagnostic.providerLimitSurfaces);
  }
  return hasProviderLimitSurface(diagnostic.dialogs);
}

function hasProviderLimitSurface(surfaces) {
  return Array.isArray(surfaces)
    && surfaces.some(surface => surface.providerLimitMatched === true
      || PROVIDER_LIMIT_RE.test(String(surface.textPreview || surface.text || '')));
}
