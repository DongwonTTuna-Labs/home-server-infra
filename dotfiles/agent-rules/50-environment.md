## Machines and Where Things Live

Two machines: the macOS laptop and the Ubuntu home server, reachable from the
laptop as `ssh home`. The rules below hold on both.

Codex reads `~/.codex/AGENTS.md`; Claude Code reads `~/.claude/CLAUDE.md`. They
are the same file, generated from one canon, so a rule stated here applies
whichever tool is running. Skills, prompts, and agent definitions are
loaded per tool — `~/.codex/skills/`, `~/.codex/prompts/`, `~/.codex/agents/`
for Codex, `~/.claude/skills/` for Claude. The hand-authored ones shared by
both machines are tracked in the same repository as this canon and installed by
the same script; vendored skills and the symlinks into `~/.agents/skills/` are
owned by their installers and are not.

Reference documents are shared. They are ordinary files, so either tool can
read them; only Claude auto-loads `~/.claude/rules/`, and Codex must open them
itself. Load one only when the decision in front of you actually needs it:

- `~/.codex/runbooks/execution-policy.md` — PR state and project memory, spec
  drift and retired attempts, planning shape, the long form of anomaly triage.
- `~/.codex/runbooks/gpt-webai-pro.md` — operating the ChatGPT Pro slot daemon.
- `~/.codex/runbooks/gptpro-review.md` — the gptpro-review stack.
- `~/.claude/rules/codex-lb-models.md` — which relay model to hand Codex, what
  each account is entitled to, how to invoke it.
- `~/.claude/rules/gws-cli.md` — any Google Workspace work (Gmail, Drive,
  Calendar, Sheets, Docs) goes through the `gws` CLI; this has the traps.
- `~/.claude/rules/routefork-current.md` — where RouteFork work happens; that
  repository's own agent files take over from there.

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
