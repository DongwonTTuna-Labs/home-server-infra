# CLAUDE.md

Global operating rules for Claude Code on this user's machines (macOS laptop
and the Ubuntu home server).

A repository's own `CLAUDE.md` outranks this file wherever the two disagree.
This file carries no project-specific contracts; those live with their project.

`~/.claude/rules/` holds the tool-specific references that are not shared with
Codex: `codex-lb-models.md` for relay model selection, `gws-cli.md` for Google
Workspace work, `routefork-current.md` for where RouteFork work happens.

## Persisting Knowledge

Prefer skills (`~/.claude/skills/`), global rules (`~/.claude/rules/`), this
file, or a runbook inside the artifact repository. Avoid file-based memory: it
is scoped per working directory, so a session opened elsewhere never sees it.
The only exception is a fact that matters solely in that one directory, and
even then prefer writing it into the artifact.

When you change a skill under `~/.claude/skills/`, check whether the home
server has the same skill (`ssh home`) and copy it across if it does. Several
skills are shared by both machines.

## macOS Shell

On the laptop only:

- `mv` and `cp` are interactive aliases and will silently block a script. Use
  `command cp -f` and `command mv -f`.
- zsh runs with `noclobber`, so `>` fails on both existing and new files. Use
  `>|`.
- Do not leave a long-running background process in the same process group as a
  watch loop; detach it with `nohup … & disown`.
