# Agent Operating Rules

Global rules for the coding agents on this user's machines. Codex loads this as
`~/.codex/AGENTS.md` and Claude Code loads it as `~/.claude/CLAUDE.md`. The two
files are byte-identical on purpose: there is one set of rules, not one per
tool.

A repository's own agent file — its `AGENTS.md` or `CLAUDE.md` — outranks this
one wherever they disagree. This file carries no project-specific contracts;
those live with their project.
