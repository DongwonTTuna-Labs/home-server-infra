import { progressPrologueAnswer, sha256Text } from './common.mjs';
import { deriveTurnId } from './contracts/r13.mjs';

export async function assistantTurns(page) {
  const turns = await page.evaluate(() => {
    const visible = node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const primary = Array.from(document.querySelectorAll('[data-message-author-role="assistant"]'));
    const nodes = primary.length > 0 ? primary : Array.from(document.querySelectorAll('main article'));
    return nodes.map((node, index) => {
      const text = (node.innerText || node.textContent || '').trim();
      return {
        dataMessageId: node.getAttribute('data-message-id') || '',
        turnIndex: index,
        domId: node.id || node.getAttribute('data-testid') || '',
        text,
        visible: visible(node),
      };
    }).filter(turn => turn.visible && turn.text).map(({ visible: _visible, ...turn }) => turn);
  }).catch(() => []);
  return turns.map(turn => ({ ...turn, textSha256: sha256Text(turn.text) }));
}

export async function userTurns(page) {
  const turns = await page.evaluate(() => {
    const visible = node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const nodes = Array.from(document.querySelectorAll('[data-message-author-role="user"]')).filter(visible);
    return nodes.map((node, index) => {
      const text = (node.innerText || node.textContent || '').trim();
      return {
        dataMessageId: node.getAttribute('data-message-id') || '',
        turnIndex: index,
        domId: node.id || node.getAttribute('data-testid') || '',
        text,
      };
    }).filter(turn => turn.text);
  }).catch(() => []);
  return turns.map(turn => ({ ...turn, textSha256: sha256Text(turn.text) }));
}

export async function generationActive(page) {
  return await page.evaluate(() => {
    return Array.from(document.querySelectorAll('button,[role="button"]')).some(node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const label = `${node.getAttribute('aria-label') || ''} ${node.getAttribute('data-testid') || ''} ${node.innerText || node.textContent || ''}`;
      return /stop generating|stop responding|stop answering|stop-button|중지|정지/i.test(label);
    });
  }).catch(() => false);
}

export async function turnEvidence(page, baseline = null) {
  const users = await userTurns(page);
  const assistants = await assistantTurns(page);
  const active = await generationActive(page);
  const evidence = {
    userCount: users.length,
    assistantCount: assistants.length,
    activeTurn: active,
    latestUserSha256: users[users.length - 1]?.textSha256 || '',
    latestAssistantSha256: assistants[assistants.length - 1]?.textSha256 || '',
  };
  if (baseline) {
    evidence.baselineUserCount = Number(baseline.userCount || 0);
    evidence.baselineAssistantCount = Number(baseline.assistantCount || 0);
    evidence.newUserTurn = evidence.userCount > evidence.baselineUserCount;
    evidence.newAssistantTurn = evidence.assistantCount > evidence.baselineAssistantCount;
  }
  return evidence;
}

export async function waitForTurnEvidence(page, baseline, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let evidence = await turnEvidence(page, baseline);
  while (Date.now() < deadline) {
    evidence = await turnEvidence(page, baseline);
    if (evidence.activeTurn || evidence.newUserTurn || evidence.newAssistantTurn) return evidence;
    await page.waitForTimeout(500);
  }
  return evidence;
}

export async function r13TurnSnapshot(page, sessionId) {
  const observations = await r13TurnObservations(page);
  const map = authorRole => observations
    .filter(item => item.authorRole === authorRole)
    .map(item => ({
      ...item,
      turnId: item.dataMessageId === null
        ? null
        : deriveTurnId(sessionId, authorRole, item.dataMessageId),
    }));
  const users = map('user');
  const assistants = map('assistant');
  return {
    observations,
    users,
    assistants,
    latestUser: users.at(-1) ?? null,
    latestAssistant: assistants.at(-1) ?? null,
    missingIdentityCount: observations.filter(item => item.dataMessageId === null).length,
  };
}

export async function r13TurnObservations(page) {
  const observations = await page.evaluate(() => {
    const visible = node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    return Array.from(document.querySelectorAll('[data-testid^="conversation-turn"]'))
      .map((article, articleIndex) => {
        const message = article.matches('[data-message-author-role]')
          ? article
          : article.querySelector('[data-message-author-role]');
        if (!message || !visible(article) || !visible(message)) return null;
        const authorRole = message.getAttribute('data-message-author-role');
        if (authorRole !== 'user' && authorRole !== 'assistant') return null;
        return {
          articleIndex,
          authorRole,
          dataMessageId: message.getAttribute('data-message-id') || null,
          text: (message.innerText || message.textContent || '').trim(),
        };
      })
      .filter(Boolean);
  }).catch(() => []);
  return observations.map(({ text, ...item }) => ({
    ...item,
    textSha256: text ? `sha256:${sha256Text(text)}` : null,
  }));
}

export async function waitForR13TurnStart(page, sessionId, baseline, timeoutMs) {
  const baselineUserIds = new Set(baseline?.users?.map(item => item.dataMessageId).filter(Boolean) ?? []);
  const baselineAssistantIds = new Set(
    baseline?.assistants?.map(item => item.dataMessageId).filter(Boolean) ?? [],
  );
  const deadline = Date.now() + timeoutMs;
  let snapshot = await r13TurnSnapshot(page, sessionId);
  while (Date.now() < deadline) {
    snapshot = await r13TurnSnapshot(page, sessionId);
    const userId = snapshot.latestUser?.dataMessageId;
    const assistantId = snapshot.latestAssistant?.dataMessageId;
    if (userId && assistantId
        && !baselineUserIds.has(userId)
        && !baselineAssistantIds.has(assistantId)) {
      return { ...snapshot, confirmed: true };
    }
    await page.waitForTimeout(250);
  }
  return { ...snapshot, confirmed: false };
}

export function conversationHydrated(state) {
  return Boolean(
    state?.activeTurn
    || Number(state?.assistantCount || 0) > 0
    || Number(state?.userCount || 0) > 0
  );
}

export async function waitForConversationHydration(page, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let state = await latestAnswerState(page);
  while (!conversationHydrated(state) && Date.now() < deadline) {
    await page.waitForTimeout(500);
    state = await latestAnswerState(page);
  }
  return state;
}

export async function latestAnswerState(page) {
  const turns = await assistantTurns(page);
  const active = await generationActive(page);
  const last = turns[turns.length - 1] || null;
  const evidence = await turnEvidence(page);
  const answerText = last?.text || '';
  const prologue = progressPrologueAnswer(answerText);
  const status = active || prologue || !answerText ? 'running' : 'done';
  return {
    status,
    reason: prologue ? 'answer.progress_prologue' : undefined,
    activeTurn: active,
    assistantCount: turns.length,
    userCount: evidence.userCount,
    answerText,
    assistantTurn: last ? {
      turnIndex: last.turnIndex,
      domId: last.domId,
      textSha256: last.textSha256,
    } : undefined,
    turnEvidence: evidence,
  };
}
