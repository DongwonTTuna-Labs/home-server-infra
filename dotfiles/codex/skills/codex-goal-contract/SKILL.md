---
name: codex-goal-contract
description: Create, harden, review, or activate Codex Goals as evidence-based completion contracts. Use when asked to draft a /goal, set an active goal, revise goal wording, convert vague work into a goal, design GPT Pro or ChatGPT Pro orchestration goals, or enforce OpenAI Using Goals in Codex best practices.
---

# Codex Goal Contract

Create Goals as thread-scoped completion contracts, not loose TODO lists. A good Goal defines what finished means, what evidence proves it, and when the work must stop as blocked instead of pretending success.

Official reference: https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex

## When To Use

Use this skill for requests like:

- "goal 만들어줘"
- "/goal 설정"
- "goal 수정"
- "GPT Pro goal 설계"
- "completion contract로 바꿔줘"
- "make this a strong Codex goal"

Do not use a Goal for one-line edits, simple explanations, short code reviews, or questions where the user wants one answer and then a stop.

## Workflow

1. Ground the Goal in current context.
   - Inspect relevant files, docs, plans, repo state, active goal state, or prior evidence when available.
   - Ask only for details that cannot be discovered and materially change the contract.
   - If official OpenAI Goal guidance is material and currentness matters, verify the official page before relying on memory.

2. Draft the Goal as a compact contract with these parts:
   - Objective: the outcome to achieve.
   - Success criteria: concrete conditions that must be true.
   - Verification evidence: files, tests, logs, benchmark output, artifacts, reports, PR/check state, or research evidence to inspect.
   - Constraints and non-goals: boundaries, forbidden actions, compatibility rules, and scope exclusions.
   - Iteration policy: what to do between attempts and what evidence to record.
   - Blocked/stop conditions: when to stop, what to report, and what input is needed next.
   - Completion rule: complete only after comparing the objective to concrete evidence.

3. Keep the Goal auditable.
   - Narrow enough to verify.
   - Broad enough for Codex to choose the next useful action.
   - Honest about unavailable data, flaky benchmarks, proxy evidence, and remaining uncertainty.
   - Separate long plans into supporting files when needed; keep the active Goal focused on the contract.

4. If the user asks to actually set the Goal:
   - Call `get_goal` first when an active goal might exist.
   - Call `create_goal` only when the user explicitly asks to create or set an active Goal.
   - Put the full contract text in the goal objective.
   - Do not set `token_budget` unless the user explicitly requested a budget.
   - Do not overwrite, clear, pause, resume, or fake goal state.

5. When finishing Goal work:
   - Use `update_goal(status="complete")` only when every success criterion has concrete supporting evidence.
   - Use `update_goal(status="blocked")` only after the same blocker has repeated for the required consecutive turns and no meaningful progress remains without user input or external state change.
   - Reaching a budget or tool timeout is not completion.

## Mandatory Guardrails

- Never directly merge PRs.
- For GPT Pro or ChatGPT Pro orchestration Goals, include this rule directly in the Goal text: Pro work may take 3+ hours; wait unconditionally and never cancel, interrupt, refresh, resubmit, switch sessions, or treat MCP polling timeouts as failure.
- While waiting for Pro, do not click around, run snapshots, doctor sessions, switch tabs, retry in another UI, or otherwise disturb the session unless the user explicitly instructs it.
- For Goals involving external model, browser, computer, or other external-tool automation, include this rule directly in the Goal text: MCP tools only; all GPT Pro, browser, computer, and external tool interaction must go through available MCP tools.
- If an MCP wait or polling call times out while Pro is still expected to work, continue waiting on the same session without UI disturbance unless the user gives a new instruction.

## Output Shape

For a draft, return a concise Goal body the user can paste into `/goal` or that can be passed to `create_goal`.

For actual goal activation, first summarize the exact contract being activated, then call the goal tool. After tool success, report the active Goal constraints that matter operationally.

## Strong Goal Template

```text
Objective:
<specific outcome>

Success criteria:
- <condition that must be true>
- <condition that must be true>

Verification evidence:
- <tests/logs/files/artifacts/checks to inspect>

Constraints and non-goals:
- <forbidden action or scope boundary>
- Never directly merge PRs.

Iteration policy:
- After each attempt, record what changed, what evidence showed, and the next best action.

Blocked/stop conditions:
- If <specific blocker>, stop with attempted paths, evidence gathered, blocker, and next input needed.

Completion rule:
- Mark complete only after checking the objective against the verification evidence.
```

## GPT Pro Goal Addendum

Append this to any GPT Pro or ChatGPT Pro orchestration Goal:

```text
GPT Pro waiting rule:
Pro work may take 3+ hours. Wait unconditionally. Do not cancel, interrupt, refresh, resubmit, switch sessions, open diagnostic snapshots, run doctor tools, click around, retry in another UI, or treat MCP polling timeouts as failure unless the user explicitly instructs it.

MCP-only rule:
Use MCP tools only for GPT Pro, browser, computer, and external-tool interaction.
```
