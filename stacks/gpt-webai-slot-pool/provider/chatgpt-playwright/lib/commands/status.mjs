import {
  jsonOut,
} from '../common.mjs';
import {
  classifyReadiness,
  selectPage,
  withBrowser,
} from '../browser.mjs';
import { classifyReadinessSignals } from '../readiness.mjs';
import { captureDiagnostics } from './shared.mjs';

export function statusStateFromDiagnostics(diagnostics = {}) {
  const signals = diagnostics.readinessSignals;
  if (!signals) return null;
  return classifyReadinessSignals({
    url: diagnostics.url || '',
    login: signals.login,
    challenge: signals.challenge,
    providerLimit: signals.providerLimit ?? signals.limit,
    upgrade: signals.upgrade,
    pro: signals.pro,
    composer: signals.composer,
    send: signals.send,
  });
}

export function statusPayloadFromSnapshot({ diagnostics, fallbackState, pageUrl = '' } = {}) {
  const state = statusStateFromDiagnostics(diagnostics) || fallbackState || {};
  return {
    ok: true,
    vendor: 'chatgpt',
    status: state.status,
    reason: state.reason || undefined,
    url: state.url || diagnostics?.url || pageUrl,
    reachable: true,
    headed: true,
    composer: state.composer,
    send: state.send,
    pro: state.pro,
    upgrade: state.upgrade,
    diagnostics,
  };
}

export async function commandStatus() {
  await withBrowser(async browser => {
    const page = await selectPage(browser);
    const diagnostics = await captureDiagnostics(page, 'status', '');
    const fallbackState = diagnostics?.readinessSignals ? null : await classifyReadiness(page);
    jsonOut(statusPayloadFromSnapshot({
      diagnostics,
      fallbackState,
      pageUrl: page.url(),
    }));
  });
}

export async function handleStatus(context, overrides = {}) {
  const dependencies = { classifyReadiness, ...overrides };
  try {
    const state = await dependencies.classifyReadiness(context.page);
    const composerReady = state.composer === true;
    const modelLabel = state.pro === true ? 'pro' : composerReady ? 'non_pro' : 'unknown';
    let healthStatus = state.status;
    if (composerReady && modelLabel === 'non_pro'
        && !['login_required', 'provider_limit'].includes(healthStatus)) {
      healthStatus = 'ready_model_correction_required';
    }
    if (![
      'ready', 'ready_model_correction_required', 'login_required',
      'subscription_required', 'provider_limit', 'unreachable',
      'schema_drift', 'unknown',
    ].includes(healthStatus)) healthStatus = 'unknown';
    return {
      ok: true,
      status: 'done',
      providerReason: null,
      operationData: {
        composerReady,
        dockerStatus: 'running',
        healthStatus,
        modelLabel,
        retryAfterMs: null,
      },
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      ok: false,
      status: 'failed',
      providerReason: /timeout/i.test(message) ? 'probe.timeout' : 'probe.unreachable',
      operationData: {
        composerReady: false,
        dockerStatus: 'unknown',
        healthStatus: 'unreachable',
        modelLabel: 'unknown',
        retryAfterMs: null,
      },
    };
  }
}
