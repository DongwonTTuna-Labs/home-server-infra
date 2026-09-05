import type { Page } from "playwright-core";
import { GwpError } from "../../shared/errors.js";
import { sha256Text } from "../../shared/fsx.js";
import type { PollParams, PollResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import { generatedImageControls, generatedImagesLoaded, imageAnswerActionVisible } from "./images.js";
import {
  assistantAfter,
  answerActionVisible,
  artifactControls,
  FILENAME_PATTERN,
  generationActive,
  readAssistantAnswer,
  readTurns,
  type TurnObservation,
} from "../selectors.js";
const STABLE_GAP_MS = 3_000;
const ARTIFACT_GRACE_MS = 8_000, ARTIFACT_POLL_MS = 500;
const EMPTY_ARTIFACT_ANSWER = "\u0000";
const ARTIFACT_HINT = /\b(?:download|downloadable|file|files|attachment|attached)\b|다운로드|파일|첨부/iu;
export async function pollConversation(
  session: BrowserSession,
  params: PollParams,
  openConversation = (url: string) => session.open(url),
): Promise<PollResult> {
  if (!Number.isInteger(params.waitMs) || params.waitMs < 0 || params.waitMs > 60_000) {
    throw new Error("poll waitMs must be an integer from 0 through 60000");
  }
  if (!/^[0-9a-f]{64}$/.test(params.promptSha256)) {
    throw new Error("poll promptSha256 must be 64 lower-hex characters");
  }
  const page = await bindPollPage(session, params, openConversation);
  if (params.imageCount !== undefined) return pollImages(page, params);
  const deadline = Date.now() + params.waitMs;
  let stableText = "";
  let stableSince = 0;
  let observedAssistantTurnId: string | undefined;
  do {
    const turns = await readTurns(page);
    const user = matchingUser(turns, params);
    const assistant = matchingAssistant(turns, user, params.assistantTurnId);
    observedAssistantTurnId = assistant?.dataMessageId;
    const active = await generationActive(page);
    const answerText = assistant ? await readAssistantAnswer(page, assistant.dataMessageId) : "";
    const artifactHint = Boolean(assistant && hasArtifactHint(assistant.text, answerText));
    const stabilityText = answerText || (artifactHint ? assistant?.text || EMPTY_ARTIFACT_ANSWER : "");
    const actionsReady = assistant && stabilityText
      ? await answerActionVisible(page, assistant.dataMessageId) : false;
    if (!active && assistant && stabilityText && actionsReady) {
      if (stabilityText === stableText) {
        if (stableSince > 0 && Date.now() - stableSince >= STABLE_GAP_MS) {
          const controls = await settledArtifactControls(
            page, assistant.dataMessageId, artifactHint, deadline,
          );
          if (!controls || (!controls.length && !answerText)) break;
          const finalAnswer = controls.length > 0
            ? stripArtifactActionLines(await readAssistantAnswer(page, assistant.dataMessageId))
            : answerText;
          return {
            state: "complete",
            currentUrl: page.url(),
            assistantTurnId: assistant.dataMessageId,
            answerMarkdown: finalAnswer,
            answerSha256: sha256Text(finalAnswer),
            artifactControls: controls,
          };
        }
      } else {
        stableText = stabilityText;
        stableSince = Date.now();
      }
    } else {
      stableText = "";
      stableSince = 0;
    }
    if (Date.now() >= deadline) break;
    await page.waitForTimeout(Math.min(250, Math.max(1, deadline - Date.now())));
  } while (Date.now() <= deadline);
  return { state: "generating", currentUrl: page.url(),
    ...(observedAssistantTurnId ? { assistantTurnId: observedAssistantTurnId } : {}) };
}
async function pollImages(page: Page, params: PollParams): Promise<PollResult> {
  if (!params.userTurnId || !Number.isInteger(params.imageCount) || params.imageCount! < 1 || params.imageCount! > 5) {
    throw new GwpError("turn_not_found", "image poll requires a confirmed user turn and imageCount from 1 through 5");
  }
  const deadline = Date.now() + params.waitMs;
  let stableCount = -1;
  let stableAnswer = "";
  let stableSince = 0;
  do {
    const controls = await generatedImageControls(page, params.userTurnId);
    const turns = await readTurns(page);
    const assistant = matchingAssistant(turns, matchingUser(turns, params), params.assistantTurnId);
    const answer = assistant ? await readAssistantAnswer(page, assistant.dataMessageId) : "";
    const complete = !await generationActive(page)
      && await imageAnswerActionVisible(page, controls, assistant?.dataMessageId)
      && await generatedImagesLoaded(page, controls)
      // 단일 이미지 재개 시에는 갤러리 이름도 아직 없는 빈 응답이 먼저 보인다.
      && (controls.length > 0 || Boolean(answer.trim()));
    if (complete) {
      if (controls.length !== stableCount || answer !== stableAnswer) {
        stableCount = controls.length; stableAnswer = answer; stableSince = Date.now();
      }
      else if (Date.now() - stableSince >= (controls.length ? STABLE_GAP_MS : ARTIFACT_GRACE_MS)) return {
        state: "complete", currentUrl: page.url(), answerMarkdown: answer, answerSha256: sha256Text(answer),
        ...(assistant?.dataMessageId ? { assistantTurnId: assistant.dataMessageId } : {}),
        artifactControls: controls.map((_, index) => ({ index, label: `Generated image ${index + 1}` })),
      };
    } else { stableCount = -1; stableSince = 0; }
    if (Date.now() >= deadline) break;
    await page.waitForTimeout(Math.min(250, Math.max(1, deadline - Date.now())));
  } while (Date.now() <= deadline);
  return { state: "generating", currentUrl: page.url() };
}
async function settledArtifactControls(
  page: Page, assistantTurnId: string, artifactHint: boolean, pollDeadline: number,
) {
  let controls = await artifactControls(page, assistantTurnId);
  if (controls.length > 0 || !artifactHint) return controls;
  const graceDeadline = Date.now() + ARTIFACT_GRACE_MS;
  const deadline = Math.min(pollDeadline, graceDeadline);
  while (Date.now() < deadline) {
    await page.waitForTimeout(Math.min(ARTIFACT_POLL_MS, deadline - Date.now()));
    controls = await artifactControls(page, assistantTurnId);
    if (controls.length > 0) break;
  }
  return controls.length > 0 || pollDeadline >= graceDeadline ? controls : null;
}
function hasArtifactHint(rawAnswer: string, answer: string): boolean {
  return !answer.trim() || FILENAME_PATTERN.test(rawAnswer) || ARTIFACT_HINT.test(rawAnswer);
}
function stripArtifactActionLines(answer: string): string {
  return answer.split(/\r?\n/gu).filter((line) => !/^\s*(?:download|다운로드)\s*[.!]?\s*$/iu.test(line))
    .join("\n").trim();
}
async function bindPollPage(
  session: BrowserSession,
  params: PollParams,
  openConversation: (url: string) => Promise<Page>,
): Promise<Page> {
  const current = await session.inspectionPage();
  const relevant = await session.relevantPages();
  const pages = current
    ? [current, ...relevant.filter((page) => page !== current)]
    : relevant;
  const byTurnId = await findPage(pages, (turns) => hasConfirmedTurn(turns, params));
  if (byTurnId) return byTurnId;
  const storedPage = pages.find((page) => page.url() === params.conversationUrl);
  if (storedPage) return storedPage;
  let opened: Page;
  try {
    opened = await openConversation(params.conversationUrl);
  } catch (error) {
    const fallback = await findFallbackPage(session, params);
    if (fallback) return fallback;
    throw error;
  }
  if (session.isConversationUrl(opened.url())) return opened;
  const fallback = await findFallbackPage(session, params);
  if (fallback) return fallback;
  throw new GwpError(
    "turn_not_found",
    "stored conversation URL redirected outside /c/ and no open tab matched the request",
  );
}
async function findFallbackPage(
  session: BrowserSession,
  params: PollParams,
): Promise<Page | null> {
  const fallbackPages = await session.relevantPages();
  const fallbackByTurnId = await findPage(fallbackPages, (turns) => (
    hasConfirmedTurn(turns, params)
  ));
  if (fallbackByTurnId) return fallbackByTurnId;
  const fallbackByPrompt = await findPages(fallbackPages, (turns) => (
    Boolean(matchingPromptUser(turns, params))
  ));
  if (fallbackByPrompt.length > 1) {
    throw new GwpError(
      "turn_not_found",
      "multiple open conversations matched the prompt without a durable turn or URL anchor",
    );
  }
  return fallbackByPrompt[0] ?? null;
}
async function findPage(
  pages: readonly Page[],
  matches: (turns: TurnObservation[]) => boolean,
): Promise<Page | null> {
  return (await findPages(pages, matches))[0] ?? null;
}
async function findPages(
  pages: readonly Page[],
  matches: (turns: TurnObservation[]) => boolean,
): Promise<Page[]> {
  const found: Page[] = [];
  for (const page of pages) {
    const turns = await readTurns(page).catch(() => []);
    if (matches(turns)) found.push(page);
  }
  return found;
}
function hasConfirmedTurn(turns: TurnObservation[], params: PollParams): boolean {
  return Boolean(params.userTurnId && turns.some((turn) => (
    turn.role === "user" && turn.dataMessageId === params.userTurnId
  )));
}
function matchingPromptUser(
  turns: TurnObservation[],
  params: PollParams,
): TurnObservation | undefined {
  return [...turns].reverse().find((turn) => (
    turn.role === "user" && sha256Text(turn.text) === params.promptSha256
  ));
}
function matchingUser(
  turns: TurnObservation[],
  params: PollParams,
): TurnObservation | undefined {
  if (params.userTurnId) {
    return turns.find((turn) => (
      turn.role === "user" && turn.dataMessageId === params.userTurnId
    ));
  }
  return matchingPromptUser(turns, params);
}
function matchingAssistant(
  turns: TurnObservation[],
  user: TurnObservation | undefined,
  assistantTurnId: string | undefined,
): TurnObservation | undefined {
  if (!user) return undefined;
  if (assistantTurnId) {
    const exact = turns.find((turn) => (
      turn.role === "assistant" && turn.dataMessageId === assistantTurnId
      && turn.domIndex > user.domIndex
    ));
    if (exact) return exact;
  }
  return assistantAfter(turns, user);
}
