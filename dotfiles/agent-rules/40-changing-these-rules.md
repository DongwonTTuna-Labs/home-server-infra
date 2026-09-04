## Changing These Rules

`~/.codex/AGENTS.md` and `~/.claude/CLAUDE.md` are generated. Editing either one
directly works until the next install overwrites it, which is how a rule added
by a parallel session gets silently lost. Write to the right place instead.

Find the canon on this machine — the installer records where it came from:

```sh
cat ~/.local/state/agent-config-source
```

That file gives the repository path. From there:

- **A durable rule for every session and both tools** — edit
  `dotfiles/agent-rules/*.md`, run `dotfiles/bin/build-agent-docs.py`, then
  `dotfiles/bin/install-agent-config.sh`, and commit. This is the normal path.
- **A Claude-only reference** — add `~/.claude/rules/<topic>.md`. That directory
  loads automatically and the installer only manages the three files it lists,
  so a new file there survives regeneration. Say so in your report and promote
  it to the canon when it proves durable.
- **A rule about one project** — put it in that repository's own `AGENTS.md` or
  `CLAUDE.md`, where it outranks the global file and loads only there.
- **Something still unverified** — do not write it into the canon yet. Report it
  and let the user decide.

If the canon is genuinely unreachable, do not hand-edit the generated file.
Write the Claude-only note or tell the user what you would have added, name the
file it belongs in, and stop there.

Several sessions run in parallel on these machines. A change you cannot see
being made is still being made, so prefer the durable location over the fast
one, and never treat a generated file as a scratchpad.
