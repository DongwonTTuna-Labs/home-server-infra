import { conversationIdFromUrl, validConversationUrl } from './common.mjs';
import { deriveTurnId } from './contracts/r13.mjs';
import { r13TurnObservations, turnEvidence } from './turns.mjs';

const START_CONFIRMATION_POLL_MS = 500;

function hasStartEvidence(evidence) {
  return Boolean(evidence?.activeTurn || evidence?.newUserTurn || evidence?.newAssistantTurn);
}

export async function waitForSendStartConfirmation(page, baseline, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let conversationUrl = page.url();
  let evidence = await turnEvidence(page, baseline);

  while (Date.now() < deadline) {
    conversationUrl = page.url();
    evidence = await turnEvidence(page, baseline);
    if (validConversationUrl(conversationUrl) && hasStartEvidence(evidence)) {
      return { conversationUrl, turnEvidence: evidence };
    }
    await page.waitForTimeout(START_CONFIRMATION_POLL_MS);
  }

  conversationUrl = page.url();
  evidence = await turnEvidence(page, baseline);
  return { conversationUrl, turnEvidence: evidence };
}

export async function waitForR13SendStartConfirmation(page, baseline, timeoutMs) {
  const baselineIds = new Set(
    (baseline || []).map(item => item.dataMessageId).filter(Boolean),
  );
  const deadline = Date.now() + timeoutMs;
  let last = { conversationUrl: page.url(), observations: [] };
  while (Date.now() < deadline) {
    const conversationUrl = page.url();
    const sessionId = conversationIdFromUrl(conversationUrl);
    const observations = await r13TurnObservations(page);
    last = { conversationUrl, observations };
    if (sessionId) {
      const { user, assistant } = selectR13TurnStartPair(observations, baselineIds);
      if (user && assistant) {
        return {
          confirmed: true,
          conversationUrl,
          sessionId,
          userTurnId: deriveTurnId(sessionId, 'user', user.dataMessageId),
          assistantTurnId: deriveTurnId(sessionId, 'assistant', assistant.dataMessageId),
          user,
          assistant,
          observations,
        };
      }
    }
    await page.waitForTimeout(250);
  }
  return { ...last, confirmed: false, sessionId: '', userTurnId: null, assistantTurnId: null };
}

export function selectR13TurnStartPair(observations, baselineIds = new Set()) {
  const newTurns = (observations || []).filter(item => (
    item?.dataMessageId
    && Number.isInteger(item.articleIndex)
    && !baselineIds.has(item.dataMessageId)
  ));
  const users = newTurns
    .filter(item => item.authorRole === 'user')
    .sort((left, right) => right.articleIndex - left.articleIndex);
  const assistants = newTurns
    .filter(item => item.authorRole === 'assistant')
    .sort((left, right) => left.articleIndex - right.articleIndex);
  for (const user of users) {
    const assistant = assistants.find(item => item.articleIndex > user.articleIndex);
    if (assistant) return { user, assistant };
  }
  return { user: null, assistant: null };
}

export async function reconcileR13TurnStart(page, promptSha256, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let last = { conversationUrl: page.url(), observations: [] };
  while (Date.now() < deadline) {
    const conversationUrl = page.url();
    const sessionId = conversationIdFromUrl(conversationUrl);
    const observations = await r13TurnObservations(page);
    last = { conversationUrl, observations };
    if (sessionId) {
      const user = observations
        .filter(item => item.authorRole === 'user'
          && item.dataMessageId
          && item.textSha256 === promptSha256)
        .at(-1);
      const assistant = observations
        .filter(item => item.authorRole === 'assistant'
          && item.dataMessageId
          && user
          && item.articleIndex > user.articleIndex)
        .at(-1);
      if (user && assistant) {
        return {
          confirmed: true,
          conversationUrl,
          sessionId,
          userTurnId: deriveTurnId(sessionId, 'user', user.dataMessageId),
          assistantTurnId: deriveTurnId(sessionId, 'assistant', assistant.dataMessageId),
          user,
          assistant,
          observations,
        };
      }
    }
    await page.waitForTimeout(250);
  }
  return { ...last, confirmed: false, sessionId: '', userTurnId: null, assistantTurnId: null };
}
