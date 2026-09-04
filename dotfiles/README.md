# dotfiles

Agent instruction files and Codex configuration for the two machines that run
agents: the macOS laptop and the Ubuntu home server.

## Why this exists

The global agent rules used to live only in `~/.codex` and `~/.claude` on each
machine. They drifted: skills differed 8 versus 4, `gpt-webai-pro.md` differed
by a thousand bytes, the two `CLAUDE.md` files had diverged, and the same rules
existed in three places with three slightly different wordings. Nothing pointed
at which copy was right.

Now there is one canon in git, and the per-tool files are generated from it.

## Layout

```
agent-rules/            the canon — tool-agnostic rules, one topic per file
  00-core.md            communication, invariants, autonomy, evidence, scope, reporting
  10-engineering.md     anomalies, tests, implementation, pushing
  20-delegation.md      subagents, GPT/ChatGPT delegation
  30-repo-artifacts.md  no home-grown integrity layers
  40-changing-these-rules.md  where to write a rule change, and where not to
  adapters/
    codex-header.md     Codex-only preamble (runbook index, precedence)
    claude-header.md    Claude-only preamble (rules dir, memory policy, macOS shell)

codex/
  AGENTS.md             GENERATED -> ~/.codex/AGENTS.md
  runbooks/             conditional runbooks -> ~/.codex/runbooks/
    execution-policy.md PR state, planning, anomaly triage, completion
    gpt-webai-pro.md    ChatGPT Pro slot daemon, client side
    gptpro-review.md    the gptpro-review stack
  config.toml           home-server Codex config baseline
  rules/default.rules   Codex command allowlist

claude/
  CLAUDE.md             GENERATED -> ~/.claude/CLAUDE.md
  rules/                Claude-only references -> ~/.claude/rules/

bin/
  build-agent-docs.py   canon + adapter header -> the two generated files
  install-agent-config.sh  install onto this machine
```

Neither Codex nor Claude Code can include another file at load time, so the
shared rules have to be physically present in both `AGENTS.md` and `CLAUDE.md`.
Generating them is what keeps them identical.

## Three layers

1. **Always loaded** — `AGENTS.md` / `CLAUDE.md`. Decisions every session
   needs. No project-specific contracts.
2. **Conditional runbooks** — `~/.codex/runbooks/`. Loaded when the decision in
   front of the agent needs them.
3. **Project-local** — an `AGENTS.md` in the repository it governs, such as
   `stacks/gpt-webai-pro/AGENTS.md`. It outranks the global file.

A rule that belongs to one project does not go in layer 1. That is how the
global file ended up 40% about a single stack.

## Editing

Edit the canon, never the generated files:

```sh
$EDITOR dotfiles/agent-rules/00-core.md
dotfiles/bin/build-agent-docs.py          # regenerate
dotfiles/bin/install-agent-config.sh      # install onto this machine
```

Then do the same on the other machine after pulling. `scripts/verify-layout.sh`
fails if a generated file does not match the canon, so a hand-edit of
`codex/AGENTS.md` is caught rather than silently overwritten later.

`install-agent-config.sh --dry-run` prints what it would do. Replaced files are
copied to `~/.local/state/agent-config-backups/<timestamp>/` before being
overwritten, and retired files are backed up there before removal.

Several agent sessions run on these machines at once, so the installer records
a digest of everything it wrote in `~/.local/state/agent-config/installed-manifest`
and **refuses to run** if a managed file changed after the last install — that
is almost always another session having added a rule to a generated file. Move
the edit into the canon, then rerun; `--force` overwrites and is only correct
once the edit is already preserved. The installer also writes
`~/.local/state/agent-config-source` so any session can find the canon on this
machine without guessing the path.

## Retired by this layout

`~/.codex/runbooks/codex-agent-policy.md` and
`~/.codex/runbooks/controlled-boldness.md` are merged into
`codex/runbooks/execution-policy.md`; the parts that duplicated the global file
were dropped rather than copied. `~/AGENTS.md` on the home server is gone — the
subagent depth limit moved into the canon and the `gptpro-review` section
belongs to that repository. `~/.claude/rules/gpt-delegation.md` and
`no-custom-integrity-layers.md` are now canon sections shared with Codex.

## Not managed here

Codex and Claude Code rewrite parts of their own configuration at runtime.
`[projects]`, `[hooks.state]`, `[tui.*]`, and `[marketplaces.*]` in
`config.toml` are runtime-managed and intentionally untracked; `config.toml`
here is the home server's operator-owned baseline, and the laptop differs in two
documented ways — it reaches the relay over
`https://relay-ai.dongwontuna.net/backend-api/codex` with
`CODEX_LB_LOCAL_API_KEY` instead of loopback with `CODEX_LB_HOME_API_KEY`.

Skill directories (`~/.codex/skills/`, `~/.claude/skills/`) are not yet
tracked. They are still out of sync between the machines — the laptop's Codex
skills are a superset of the home server's — and unifying them is a separate
job. `~/.codex/prompts/` and `~/.codex/agents/` are likewise untracked, as is
the `runbooks/gpt-webai-pro-deepresearch/` directory.

The two Korean runbooks were adopted verbatim rather than translated. The
laptop's `gpt-webai-pro.md` was a strict superset of the home server's — seven
extra lines recording the 2026-09-01 recovery fix — so the laptop copy became
canonical and the home server gained those lines.
