import { scrollBottomProofReason, scrollBottomProofVerified } from '../scroll-proof.mjs';
import { targetIdForUrl } from './shared.mjs';

export function latestDiagnostic(diagnostics = []) {
  const values = Array.isArray(diagnostics) ? diagnostics.filter(Boolean) : [];
  return values[values.length - 1] || null;
}

export function latestDiagnosticWithScrollProof(diagnostics = []) {
  return [...(Array.isArray(diagnostics) ? diagnostics : [])]
    .reverse()
    .find(diagnostic => diagnostic?.scrollBottomProof);
}

export function hasVerifiedBottomScrollDiagnostics(diagnostics = []) {
  const diagnostic = latestDiagnostic(diagnostics);
  return Boolean(diagnostic && scrollBottomProofVerified(diagnostic));
}

export function hasUnverifiedBottomScrollDiagnostics(diagnostics = []) {
  const diagnostic = latestDiagnostic(diagnostics);
  return Boolean(diagnostic && !scrollBottomProofVerified(diagnostic));
}

export function scrollBottomUnverifiedPayload({
  sessionId = '',
  url = '',
  state = {},
  diagnostics = [],
  status = 'scroll.bottom_unverified',
  includeExitCode = false,
} = {}) {
  const diagnostic = latestDiagnostic(diagnostics);
  const reason = diagnostic ? scrollBottomProofReason(diagnostic) : 'scroll.bottom_unverified';
  return {
    ok: true,
    vendor: 'chatgpt',
    status,
    reason: reason || 'scroll.bottom_unverified',
    sessionId: sessionId || undefined,
    targetId: targetIdForUrl(url),
    conversationUrl: url,
    activeTurn: state.activeTurn,
    assistantCount: state.assistantCount,
    userCount: state.userCount,
    answerText: state.answerText,
    assistantTurn: state.assistantTurn,
    turnEvidence: state.turnEvidence,
    diagnostics,
    exitCode: includeExitCode ? 124 : undefined,
  };
}

export function applyScrollBottomUnverifiedStatus(payload, diagnostics = payload?.diagnostics || [], status = 'scroll.bottom_unverified') {
  if (!hasUnverifiedBottomScrollDiagnostics(diagnostics)) return false;
  const diagnostic = latestDiagnostic(diagnostics);
  payload.status = status;
  payload.reason = scrollBottomProofReason(diagnostic) || 'scroll.bottom_unverified';
  return true;
}
