## Machines and Where Things Live

Two machines: the macOS laptop and the Ubuntu home server, reachable from the
laptop as `ssh home`. The rules below hold on both.

Codex reads `~/.codex/AGENTS.md`; Claude Code reads `~/.claude/CLAUDE.md`. They
are the same file, generated from one canon, so a rule stated here applies
whichever tool is running. Each tool also has its own auxiliary directories:
`~/.codex/skills/`, `~/.codex/prompts/`, `~/.codex/agents/`, and
`~/.claude/skills/`, `~/.claude/rules/`. Read whichever belongs to the tool you
are; `~/.claude/rules/` currently holds relay model selection
(`codex-lb-models.md`), Google Workspace work (`gws-cli.md`), and where
RouteFork work happens (`routefork-current.md`).

Runbooks live in `~/.codex/runbooks/` regardless of which tool is reading them.
Load one only when the decision in front of you actually needs it:

- `execution-policy.md` — PR state and project memory, spec drift and retired
  attempts, planning shape, the long form of anomaly triage.
- `gpt-webai-pro.md` — operating the ChatGPT Pro slot daemon.
- `gptpro-review.md` — the gptpro-review stack.

## Persisting Knowledge

Prefer a skill, a rules file, this canon, or a runbook inside the artifact
repository. Avoid file-based memory: it is scoped per working directory, so a
session opened elsewhere never sees it. The only exception is a fact that
matters solely in that one directory, and even then prefer writing it into the
artifact.

When you change a skill, check whether the other machine has the same skill and
copy it across if it does. Several skills are shared by both.

## macOS Shell

On the laptop only:

- `mv` and `cp` are interactive aliases and will silently block a script. Use
  `command cp -f` and `command mv -f`.
- `diff` resolves to a shell function whose definition file is missing and
  fails with `function definition file not found`. Use `command diff`.
- zsh runs with `noclobber`, so `>` fails on both existing and new files. Use
  `>|`.
- Do not leave a long-running background process in the same process group as a
  watch loop; detach it with `nohup … & disown`.
- `rsync` is not installed. To copy a tree to the home server use
  `COPYFILE_DISABLE=1 tar cf - <paths> | ssh home 'tar xf - -C <dest>'`; without
  `COPYFILE_DISABLE` bsdtar emits AppleDouble `._name` siblings that break
  globs and text parsing on the far side.
