## Subagents

Delegate deliberately. Do not default to carrying every task alone, and do not
spawn agents for work you can finish in one edit.

Delegate when the work is broad and read-heavy — you need a conclusion, not a
pile of file contents — when independent pieces can run in parallel, or when a
fresh reviewer adds real signal. Do it yourself when you already know the file
or symbol, when it is a single change, or when writing the prompt costs more
than the work.

A delegation prompt carries: target paths, the deliverable, success criteria,
existing patterns and constraints, what not to do, and how the result will be
verified. The subagent cannot see this conversation.

Treat returned work as a draft. Verify its claims against the current files
before acting on them, and rewrite its prose before any of it reaches the user
— agent-written text routinely carries small grammatical errors.

Depth limit is 2. The agent working directly for the user is depth 0, and every
delegation prompt states `Depth: N`, the parent's depth plus one. An agent at
depth 2 completes the work itself and never delegates again. If its depth is
unclear, it does not delegate until that is resolved.

## GPT / ChatGPT Delegation

When the user says to ask GPT or ChatGPT, do not answer locally. Delegate to a
real ChatGPT Pro session through the home server slot daemon `gpt-webai-pro`,
which is the only supported path — from the Mac, over `ssh home`.

- Send whole context as a zip plus a manifest, not fragments. Never include
  secrets: an upload is publication.
- A slow answer is not a failure. A timeout returns `status:"running"` with a
  `resumeCommand`; continue that session rather than resending. Never resolve
  an uncertain send by sending again — reconcile it with `resume`.
- If a slot reports `needs_login`, report that to the user; a human completes
  the login.
- Do not treat an external model's verdict as truth. Check it against the
  current source, tests, and runtime.
- Operating detail lives in `~/.codex/runbooks/gpt-webai-pro.md`.
