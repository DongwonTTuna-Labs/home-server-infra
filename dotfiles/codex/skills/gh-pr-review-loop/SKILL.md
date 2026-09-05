---
name: gh-pr-review-loop
description: Resolve GitHub pull request review feedback end to end. Use when Codex is asked to inspect PR review comments, handle unresolved review threads, classify stale versus actionable feedback, apply valid reviewer suggestions, resolve stale threads, rerun verification, or keep a Codex/GitHub review loop moving after new comments appear.
---

# GH PR Review Loop

## Workflow

1. Ground in the actual PR state first.
   - Inspect the current branch, head SHA, PR number, unresolved review threads, check runs, and latest review comments.
   - Prefer GitHub plugin workflows when available, especially `github:gh-address-comments` for review threads and `github:gh-fix-ci` for failing checks.
   - Use `gh api graphql` when thread-level resolution state or inline review context matters.

2. Classify every unresolved thread before editing.
   - Mark a thread stale only when the current head already addresses it or the commented code no longer exists.
   - Mark a thread actionable when it reports a current bug, missing validation, compatibility issue, test gap, or valid `SUGGEST` patch.
   - If evidence is incomplete, fetch the diff, surrounding code, or check logs instead of guessing.

3. Clean up stale threads directly when authorized by the task.
   - Reply with a short explanation tied to the current head.
   - Resolve the thread after replying.
   - Keep real blockers unresolved until the fix lands and is verified.

4. Implement actionable feedback conservatively.
   - Apply valid `SUGGEST` comments when they improve correctness or clarity.
   - Preserve public API compatibility unless the PR explicitly asks to break it.
   - Keep changes scoped to the reviewed surface and avoid unrelated refactors.

5. Verify and loop.
   - Run the repo's relevant format, lint, test, and whitespace checks.
   - Re-check PR checks and unresolved threads after pushing or after local fixes are ready.
   - Continue through new valid review feedback when the user has asked for autonomous follow-through.

## Output

Report findings before summaries. Separate:

- Stale threads resolved
- Actionable feedback fixed
- Checks/tests run
- Remaining blockers or evidence gaps
