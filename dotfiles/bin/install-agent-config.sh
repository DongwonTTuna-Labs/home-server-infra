#!/usr/bin/env bash
# Install the tracked agent configuration onto this machine.
#
# Runs on both the macOS laptop and the Ubuntu home server. It regenerates the
# per-tool rule files from dotfiles/agent-rules/, installs them together with
# the conditional runbooks, the Claude-only rule files, and the hand-authored
# skills, prompts, and agent definitions that both machines share. Retired
# files are moved into a timestamped backup instead of being deleted.
#
# Several agent sessions run on these machines at once, so the installer
# records what it wrote and refuses to overwrite a managed file that changed
# afterwards — including a file that appeared inside a managed directory. That
# edit belongs in the repository; losing it silently is the failure this guard
# exists to prevent.
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

# Single files: source-relative-to-dotfiles => destination under $HOME.
managed_files=(
  "codex/AGENTS.md:.codex/AGENTS.md"
  "codex/runbooks/execution-policy.md:.codex/runbooks/execution-policy.md"
  "codex/runbooks/gpt-webai-pro.md:.codex/runbooks/gpt-webai-pro.md"
  "codex/runbooks/gptpro-review.md:.codex/runbooks/gptpro-review.md"
  "claude/CLAUDE.md:.claude/CLAUDE.md"
  "claude/rules/codex-lb-models.md:.claude/rules/codex-lb-models.md"
  "claude/rules/gws-cli.md:.claude/rules/gws-cli.md"
  "claude/rules/routefork-current.md:.claude/rules/routefork-current.md"
)

# Whole directories, mirrored: files missing from the source are removed from
# the destination (after backup). Runtime debris listed in `ignored_names` is
# neither copied, removed, nor counted as a conflict.
managed_dirs=(
  "codex/skills/codex-goal-contract:.codex/skills/codex-goal-contract"
  "codex/skills/gh-pr-review-loop:.codex/skills/gh-pr-review-loop"
  "codex/skills/home-server-ops-rollout:.codex/skills/home-server-ops-rollout"
  "codex/prompts:.codex/prompts"
  "codex/agents:.codex/agents"
  "claude/skills/adversarial-gate-loop:.claude/skills/adversarial-gate-loop"
  "claude/skills/fable-sol-loop:.claude/skills/fable-sol-loop"
)
ignored_names=("__pycache__" ".DS_Store")

# Superseded. Backed up, then removed. Directories are removed whole.
retired=(
  ".codex/runbooks/codex-agent-policy.md"
  ".codex/runbooks/controlled-boldness.md"
  ".claude/rules/gpt-delegation.md"
  ".claude/rules/no-custom-integrity-layers.md"
  "AGENTS.md"
  ".codex/skills/codex-primary-runtime"
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

# Relative paths (to $root) of the regular files under $root, ignoring debris.
list_files() {
  local root="$1" expr=()
  for n in "${ignored_names[@]}"; do expr+=(-not -name "$n" -not -path "*/$n/*"); done
  [ -d "$root" ] || return 0
  (cd "$root" && find . -type f "${expr[@]}" | sed 's|^\./||' | sort)
}

recorded_digest() {
  [ -f "$manifest" ] || return 0
  awk -v rel="$1" '$2 == rel { print $1; exit }' "$manifest"
}

manifest_has_prefix() {
  [ -f "$manifest" ] || return 1
  awk -v pre="$1/" 'index($2, pre) == 1 { found = 1; exit } END { exit !found }' "$manifest"
}

backup_path() {
  local rel="$1" src="${HOME}/$1"
  [ -e "$src" ] || return 0
  local dest="${backup_dir}/${rel}"
  run mkdir -p "$(dirname "$dest")"
  if [ -d "$src" ]; then
    run command cp -Rp "$src" "$dest"
  else
    run command cp -p "$src" "$dest"
  fi
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
for entry in "${managed_files[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  [ -f "$dest" ] || continue
  want="$(recorded_digest "$rel")"
  [ -n "$want" ] || continue
  [ "$(digest "$dest")" = "$want" ] || conflicts+=("$rel")
done
for entry in "${managed_dirs[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  [ -d "$dest" ] || continue
  # A directory we have never installed is not a conflict; it is just replaced
  # (and backed up). One we have installed must match the manifest exactly.
  manifest_has_prefix "$rel" || continue
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    want="$(recorded_digest "$rel/$f")"
    if [ -z "$want" ]; then
      conflicts+=("$rel/$f (added after install)")
    elif [ "$(digest "$dest/$f")" != "$want" ]; then
      conflicts+=("$rel/$f")
    fi
  done < <(list_files "$dest")
done

if [ "${#conflicts[@]}" -gt 0 ]; then
  say "  These managed paths changed after the last install:"
  for c in "${conflicts[@]}"; do say "    ~/${c}"; done
  if [ "$force" -eq 1 ]; then
    say "  --force given: overwriting. Copies go to ${backup_dir}."
  else
    cat >&2 <<EOF

Refusing to overwrite. Something edited these after the last install — very
likely another agent session changing a generated file or a tracked skill in
place.

Recover the edit first: diff ~/<path> against ${dotfiles}/, move whatever is
worth keeping into the repository, then rerun. Use --force only once you are
certain the edit is already preserved.
EOF
    exit 1
  fi
fi
say "  none"

say "==> Installing managed files"
for entry in "${managed_files[@]}"; do
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
  backup_path "$rel"
  run mkdir -p "$(dirname "$dest")"
  run command cp -f "$src" "$dest"
  say "  installed  ~/${rel}"
done

say "==> Installing managed directories"
for entry in "${managed_dirs[@]}"; do
  src="${dotfiles}/${entry%%:*}"
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  if [ ! -d "$src" ]; then
    printf 'missing source directory: %s\n' "$src" >&2
    exit 1
  fi
  if [ -L "$dest" ]; then
    printf 'refusing to replace a symlink with a managed directory: ~/%s\n' "$rel" >&2
    exit 1
  fi
  changed=0
  backed_up=0
  # Copy new or changed files.
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    if [ -f "$dest/$f" ] && cmp -s "$src/$f" "$dest/$f"; then continue; fi
    if [ "$backed_up" -eq 0 ] && [ -d "$dest" ]; then backup_path "$rel"; backed_up=1; fi
    run mkdir -p "$(dirname "$dest/$f")"
    run command cp -f "$src/$f" "$dest/$f"
    changed=1
  done < <(list_files "$src")
  # Remove files that are no longer in the source.
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    [ -f "$src/$f" ] && continue
    if [ "$backed_up" -eq 0 ]; then backup_path "$rel"; backed_up=1; fi
    run rm -f "$dest/$f"
    changed=1
  done < <(list_files "$dest")
  if [ "$changed" -eq 1 ]; then say "  synced     ~/${rel}/"; else say "  unchanged  ~/${rel}/"; fi
done

say "==> Retiring superseded paths"
for rel in "${retired[@]}"; do
  dest="${HOME}/${rel}"
  if [ ! -e "$dest" ]; then
    say "  absent     ~/${rel}"
    continue
  fi
  backup_path "$rel"
  run rm -rf "$dest"
  say "  retired    ~/${rel}  (backup kept)"
done

if [ "$dry_run" -eq 1 ]; then
  say "==> Dry run only; nothing was written."
  exit 0
fi

mkdir -p "$state_dir"
: >"$manifest"
for entry in "${managed_files[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  [ -f "$dest" ] || continue
  printf '%s  %s\n' "$(digest "$dest")" "$rel" >>"$manifest"
done
for entry in "${managed_dirs[@]}"; do
  rel="${entry#*:}"
  dest="${HOME}/${rel}"
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    printf '%s  %s/%s\n' "$(digest "$dest/$f")" "$rel" "$f" >>"$manifest"
  done < <(list_files "$dest")
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
