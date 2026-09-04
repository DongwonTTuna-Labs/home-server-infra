# Execution Policy Runbook

Load this when the decision involves PR state, project memory, a replaced plan,
a repeated failure, or a non-trivial task that needs a plan before the first
edit. Everything here builds on `~/.codex/AGENTS.md`; it never overrides it.

This replaces the retired `codex-agent-policy.md` and `controlled-boldness.md`,
which duplicated the global rules with three slightly different versions of the
same evidence, ambiguity, and test guidance.

## PR State and Project Memory

Working memory for a pull request lives in the PR body. Read it before acting
on anything PR-shaped. Use `.ai/PR_STATE.md` only when the body is unavailable,
and never treat `.github/PULL_REQUEST_TEMPLATE.md` as state — it is a blank
form.

Long-lived project memory may live in `docs/plan/*/memory.md`. Use only the
memory that belongs to the repository, plan, or feature in front of you, and
promote a lesson there only when it crosses PR boundaries.

## Planning Before Coding

For non-trivial work, give a compact plan before editing:

- Goal
- What the spec actually says
- Key assumptions
- Material ambiguities
- Files likely to change
- Test strategy
- Risks

Cover the full requested outcome in that plan, not the first step toward it.
For trivial work, skip the plan and do the thing.

## Anomaly Triage, Long Form

Classify before fixing:

1. implementation bug
2. test bug
3. spec mismatch
4. environment issue
5. dependency or version issue
6. flaky timing or concurrency
7. stale context contamination
8. invalid assumption

State the evidence for the class you picked, then apply the smallest fix that
addresses that cause. Never hide an anomaly behind broad mocks, skipped tests,
weakened assertions, type suppression, swallowed errors, or an unrelated
rewrite.

## Replaced Plans and Repeated Failures

Do not blend two implementation plans. When a plan is replaced, stop using the
old one, record it as retired in PR state, record the new decision, and remove
the obsolete path once that is safe.

Before adopting a strategy, check PR state and the relevant
`docs/plan/*/memory.md` for approaches already tried and retired.

When a fix fails or gets reverted, write down:

- the attempt
- why it looked plausible
- what actually failed
- do not repeat
- the correct direction, if it is known

## Mode Defaults

**Coding.** Inspect the touched flow before editing. Reuse existing helpers
before adding abstractions. Verify with the narrowest check that proves the
change.

**Research.** Fetch primary and current sources when the facts may have moved.
Separate what a source says from what you inferred. Deliver a recommendation,
not a pile of links.

**Review.** Lead with findings ordered by severity, cite exact files, lines, or
live state, and separate stale comments from actionable blockers.

**Operations.** Inspect the live machine before changing it. Prefer dry runs
and health checks before claiming a rollout worked. Report the exact services,
commands, and evidence that matter.

## Completion Checklist

Before the final response on implementation work, confirm:

- the current spec is satisfied
- no retired spec was reintroduced and no known failed approach was repeated
- test changes, if any, have a stated spec or contract reason
- the diff is focused, relevant, and complete for the approved target
- critical assumptions are stated and remaining risks are disclosed
- PR state or project memory was updated when it needed to be
