import assert from "node:assert/strict";
import test from "node:test";
import {
  renderedTurnLengthSane,
  renderedTurnMatchEvidence,
  renderedTurnMatchesPrompt,
  renderedTurnMatchesPromptLoose,
} from "../../src/daemon/selectors.js";
function bigPrompt(lines = 2_000): string {
  return Array.from({ length: lines }, (_, index) => (
    `line ${index}: 설계 리뷰 지시서 본문 segment ${index * 7} with mixed 한국어/English content`
  )).join("\n");
}
test("loose matching accepts a large rendered turn with a small mid-body divergence", () => {
  const prompt = bigPrompt();
  assert.equal(renderedTurnMatchesPrompt(prompt, prompt), true);
  // ChatGPT 렌더가 중간을 수백 자 수준으로 축약/변형한 상황 (2026-07-29 실측 모드).
  const middle = Math.floor(prompt.length / 2);
  const truncated = prompt.slice(0, middle) + prompt.slice(middle + Math.floor(prompt.length * 0.03));
  assert.equal(renderedTurnMatchesPrompt(truncated, prompt), false);
  assert.equal(renderedTurnMatchesPromptLoose(truncated, prompt), true);
});
test("loose matching stays conservative", () => {
  const prompt = bigPrompt();
  const middle = Math.floor(prompt.length / 2);
  // 길이 15% 유실은 거부한다.
  const gutted = prompt.slice(0, middle) + prompt.slice(middle + Math.floor(prompt.length * 0.15));
  assert.equal(renderedTurnMatchesPromptLoose(gutted, prompt), false);
  // 앞 1000자가 다르면 거부한다.
  const divergentHead = `X${prompt.slice(1)}`;
  assert.equal(renderedTurnMatchesPromptLoose(divergentHead, prompt), false);
  // 뒤 1000자가 다르면 거부한다.
  const divergentTail = `${prompt.slice(0, -1)}X`;
  assert.equal(renderedTurnMatchesPromptLoose(divergentTail, prompt), false);
  // 렌더가 프롬프트보다 200자 넘게 길면 거부한다.
  const inflated = `${prompt.slice(0, middle)}${"덧".repeat(300)}${prompt.slice(middle)}`;
  assert.equal(renderedTurnMatchesPromptLoose(inflated, prompt), false);
  // 소형 프롬프트(<4096 정규화 문자)에는 loose를 적용하지 않는다.
  const small = "short prompt\nwith a few lines";
  assert.equal(renderedTurnMatchesPromptLoose(small.slice(0, -3), small), false);
});
test("length sanity accepts markdown-rendered turns that diverge from head to tail", () => {
  // 실 UI 실측 모드(2026-07-29): 마크다운 렌더로 firstDiff가 초반, tailMatch=0, 길이 ~98%.
  const prompt = Array.from({ length: 1_200 }, (_, index) => (
    `**항목 ${index}**: 일반 설명 텍스트가 충분히 길게 이어지는 라인입니다 (${index * 3})`
  )).join("\n");
  const rendered = prompt.replaceAll("**", "");
  assert.equal(renderedTurnMatchesPrompt(rendered, prompt), false);
  assert.equal(renderedTurnMatchesPromptLoose(rendered, prompt), false);
  assert.equal(renderedTurnLengthSane(rendered, prompt), true);
  // 길이 85% 미만은 거부.
  assert.equal(renderedTurnLengthSane(rendered.slice(0, Math.floor(rendered.length * 0.8)), prompt), false);
  // 110% 초과도 거부.
  assert.equal(renderedTurnLengthSane(rendered + "덧".repeat(Math.ceil(prompt.length * 0.25)), prompt), false);
  // 소형 프롬프트에는 적용하지 않는다.
  assert.equal(renderedTurnLengthSane("short body", "short body prompt"), false);
});
test("match evidence reports lengths and divergence offsets", () => {
  const prompt = bigPrompt(50);
  const middle = Math.floor(prompt.length / 2);
  const truncated = prompt.slice(0, middle) + prompt.slice(middle + 40);
  const evidence = renderedTurnMatchEvidence(truncated, prompt);
  assert.match(evidence, /renderedLen=\d+ promptLen=\d+ firstDiff=\d+ tailMatch=\d+/);
  const firstDiff = Number(/firstDiff=(\d+)/.exec(evidence)?.[1]);
  // 절단점 주변에서 우연히 같은 문자가 이어질 수 있어 약간의 여유를 둔다.
  assert.ok(firstDiff >= middle - 1 && firstDiff <= middle + 60, `firstDiff=${firstDiff} middle=${middle}`);
});
