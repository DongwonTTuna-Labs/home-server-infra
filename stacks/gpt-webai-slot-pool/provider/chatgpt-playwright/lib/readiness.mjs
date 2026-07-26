export function classifyReadinessSignals(signals = {}) {
  const login = Boolean(signals.login);
  const challenge = Boolean(signals.challenge);
  const limit = Boolean(signals.providerLimit ?? signals.limit);
  const upgrade = Boolean(signals.upgrade);
  const pro = Boolean(signals.pro);
  const composer = Boolean(signals.composer);

  let status = 'unknown';
  let reason = '';
  if (login || challenge) {
    status = 'login_required';
    reason = 'auth.needs_login';
  } else if (limit) {
    status = 'provider_limit';
    reason = 'provider.limit';
  } else if (!pro || (upgrade && !composer)) {
    status = 'subscription_required';
    reason = 'auth.needs_pro';
  } else if (composer) {
    status = 'ready';
  }

  return {
    ...signals,
    providerLimit: limit,
    status,
    reason,
  };
}

export function hasProModelEvidence({ selectedText = '' } = {}) {
  return /\bPro(?:\s+Extended)?\b|GPT[-\s]?[45]/i.test(selectedText);
}

export function modelBlockedByUpgrade({ upgrade = false, proEvidence = false } = {}) {
  return Boolean(upgrade && !proEvidence);
}
