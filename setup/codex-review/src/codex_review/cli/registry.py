"""Maps each CLI area to its handler and whether it needs the loaded config."""
from __future__ import annotations

from codex_review.cli.handlers import (
    auth,
    context,
    event,
    io,
    memory,
    loop,
    oidc,
    schema,
    resolve_gate,
    review,
    techlead,
    design,
    design_chief,
    fix_dispatch,
    fix_merge,
    push,
    reentry,
    issue_fallback,
)

AREAS = [
    "auth", "oidc", "io", "event", "context", "memory", "loop", "schema",
    "resolve_gate", "review", "techlead", "design", "design_chief",
    "fix_dispatch", "fix_merge", "push", "reentry", "issue_fallback",
]

CONTEXT_COMMANDS = [
    "pr",
    "changed-lines",
    "docs",
    "openspec",
    "openspec-markdown",
    "openspec-outputs",
    "memory",
    "review",
]

AREA_COMMANDS = {"context": CONTEXT_COMMANDS}

HANDLERS = {
    "auth": auth.handle_auth,
    "oidc": oidc.handle_oidc,
    "io": io.handle_io,
    "event": event.handle_event,
    "context": context.handle_context,
    "memory": memory.handle_memory,
    "loop": loop.handle_loop,
    "schema": schema.handle_schema,
    "resolve_gate": resolve_gate.handle_resolve_gate,
    "review": review.handle_review,
    "techlead": techlead.handle_techlead,
    "design": design.handle_design,
    "design_chief": design_chief.handle_design_chief,
    "fix_dispatch": fix_dispatch.handle_fix_dispatch,
    "fix_merge": fix_merge.handle_fix_merge,
    "push": push.handle_push,
    "reentry": reentry.handle_reentry,
    "issue_fallback": issue_fallback.handle_issue_fallback,
}

# Areas whose handler also receives the loaded config.yml.
NEEDS_CONFIG = {
    "context", "resolve_gate", "review", "techlead", "design", "design_chief",
    "fix_dispatch", "fix_merge", "push", "reentry", "issue_fallback",
}
