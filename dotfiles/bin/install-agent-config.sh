#!/usr/bin/env bash
# Install the tracked agent instruction files onto this machine.
#
# Runs on both the macOS laptop and the Ubuntu home server. It regenerates the
# per-tool files from dotfiles/agent-rules/, installs them plus the conditional
# runbooks and the Claude-only rule files, and moves retired files into a
# timestamped backup instead of deleting them.
#
# Several agent sessions run on these machines at once, so the installer
# records what it wrote and refuses to overwrite a managed file that changed
# afterwards. That edit belongs in the canon; losing it silently is the failure
# this guard exists to prevent.
#
#   install-agent-config.sh [--dry-run | --force]
set -euo pipefail

dry_run=0
force=0
case "${1:-}" in
  "") ;;
  --dry-run) dry_run=1 ;;
  --force) force=1 ;;
  *)
    printf 'usage: %s [--dry-run | --force]\n' "$0" >&2
    exit 2
    ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dotfiles="$(dirname "$here")"
repo_root="$(dirname "$dotfiles")"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
state_dir="${HOME}/.local/state/agent-config"
manifest="${state_dir}/installed-manifest"
source_note="${HOME}/.local/state/agent-config-source"
backup_dir="${HOME}/.local/state/agent-config-backups/${stamp}"

# source-relative-to-dotfiles => destination under $HOME
managed=(
  "codex/AGENTS.md:.codex/AGENTS.md"
  "codex/runbooks/execution-policy.md:.codex/runbooks/execution-policy.md"
  "codex/runbooks/gpt-webai-pro.md:.codex/runbooks/gpt-webai-pro.md"
  "codex/runbooks/gptpro-review.md:.codex/runbooks/gptpro-review.md"
  "claude/CLAUDE.md:.claude/CLAUDE.md"
  "claude/rules/codex-lb-models.md:.claude/rules/codex-lb-models.md"
  "claude/rules/gws-cli.md:.claude/rules/gws-cli.md"
  "claude/rules/routefork-current.md:.claude/rules/routefork-current.md"
)

# Superseded by the shared canon. Backed up, then removed.
retired=(
  ".codex/runbooks/codex-agent-policy.md"
  ".codex/runbooks/controlled-boldness.md"
  ".claude/rules/gpt-delegation.md"
  ".claude/rules/no-custom-integrity-layers.md"
  "AGENTS.md"
)

say() { printf '%s\n' "$*"; }
run() {
  if [ "$dry_run" -eq 1 ]; then
    say "would: $*"
  else
    "$@"
  fi
}

if command -v sha256sum >/dev/null 2>&1; then
  digest() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  digest() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  printf 'no sha256 tool available\n' >&2
  exit 1
fi

recorded_digest() {
  [ -f "$manifest" ] || return 0
  awk -v rel="$1" '$2 == rel { print $1; exit }' "$manifest"
}

backup() {
  local rel="$1" src="${HOME}/$1"
  [ -e "$src" ] || return 0
  local dest="${backup_dir}/${rel}"
  run mkdir -p "$(dirname "$dest")"
  run command cp -p "$src" "$dest"
}

say "==> Regenerating the per-tool files from the shared canon"
if [ "$dry_run" -eq 1 ]; then
  python3 "$here/build-agent-docs.py" --check \
    || say "  (canon and committed files differ; a real run would rewrite them)"
else
  python3 "$here/build-agent-docs.py"
fi

say "==> Checking for edits made since the last install"
conflicts=()
for entry in "${managed[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  [ -f "$dest" ] || continue
  want="$(recorded_digest "$rel")"
  [ -n "$want" ] || continue
  if [ "$(digest "$dest")" != "$want" ]; then
    conflicts+=("$rel")
  fi
done

if [ "${#conflicts[@]}" -gt 0 ]; then
  say "  These managed files changed after the last install:"
  for rel in "${conflicts[@]}"; do
    say "    ~/${rel}"
  done
  if [ "$force" -eq 1 ]; then
    say "  --force given: overwriting. Copies go to ${backup_dir}."
  else
    cat >&2 <<EOF

Refusing to overwrite. Something edited these after the last install — very
likely another agent session adding a rule to a generated file.

Recover the edit first: diff ~/<path> against the canon under
${dotfiles}/agent-rules/, move whatever is worth keeping into the canon, then
rerun. Use --force only once you are certain the edit is already preserved.
EOF
    exit 1
  fi
fi
say "  none"

say "==> Installing managed files"
for entry in "${managed[@]}"; do
  src="${dotfiles}/${entry%%:*}"
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  if [ ! -f "$src" ]; then
    printf 'missing source: %s\n' "$src" >&2
    exit 1
  fi
  if [ -f "$dest" ] && cmp -s "$src" "$dest"; then
    say "  unchanged  ~/${rel}"
    continue
  fi
  backup "$rel"
  run mkdir -p "$(dirname "$dest")"
  run command cp -f "$src" "$dest"
  say "  installed  ~/${rel}"
done

say "==> Retiring superseded files"
for rel in "${retired[@]}"; do
  dest="${HOME}/${rel}"
  if [ ! -e "$dest" ]; then
    say "  absent     ~/${rel}"
    continue
  fi
  backup "$rel"
  run rm -f "$dest"
  say "  retired    ~/${rel}  (backup kept)"
done

if [ "$dry_run" -eq 1 ]; then
  say "==> Dry run only; nothing was written."
  exit 0
fi

mkdir -p "$state_dir"
: >"$manifest"
for entry in "${managed[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  [ -f "$dest" ] || continue
  printf '%s  %s\n' "$(digest "$dest")" "$rel" >>"$manifest"
done

# Agents that need to change a rule have to find the canon. Record where this
# install came from so they do not have to guess the path on this machine.
cat >"$source_note" <<EOF
# Where the installed agent rules come from. Written by
# dotfiles/bin/install-agent-config.sh; do not edit by hand.
repo=${repo_root}
canon=${dotfiles}/agent-rules
build=${dotfiles}/bin/build-agent-docs.py
install=${dotfiles}/bin/install-agent-config.sh
installed_at=${stamp}
EOF

say "==> Backups: ${backup_dir}"
say "==> Source recorded in ${source_note}"
