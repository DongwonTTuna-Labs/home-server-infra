#!/usr/bin/env bash
# Keep agent-hermes on the newest tagged hermes-agent release.
#
# Watchtower cannot do this job. /opt/agent-apps pins images as tag@sha256, an
# immutable reference Watchtower can never see an update for, and the only
# floating tags upstream publishes are `latest` and `main` — which resolve to
# the same digest, so they track unreleased branch builds rather than releases.
# This resolves the newest vYYYY.M.D[.N] tag instead and rewrites the pin.
#
# /opt/agent-apps is root-owned and mode 0640, so the rewrite goes through a
# throwaway container that bind-mounts the stack directory. Everything else
# runs as the invoking user, who only needs read access plus the docker socket.
set -Eeuo pipefail

readonly REPO=nousresearch/hermes-agent
readonly STACK_DIR=/opt/agent-apps
readonly COMPOSE_FILE="$STACK_DIR/compose.yml"
readonly ENV_FILE="$STACK_DIR/.env"
readonly ENV_KEY=HERMES_IMAGE
readonly PROJECT=agent-apps
readonly SERVICE=hermes
readonly CONTAINER=agent-hermes
readonly HUB=https://hub.docker.com/v2/repositories
readonly TAG_RE='^v[0-9]{4}\.[0-9]+\.[0-9]+(\.[0-9]+)?$'

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/hermes-update-latest"
HEALTH_TIMEOUT="${HERMES_UPDATE_HEALTH_TIMEOUT:-300}"
SMOKE_TIMEOUT="${HERMES_UPDATE_SMOKE_TIMEOUT:-120}"

usage() {
  cat <<'EOF'
Usage: hermes-update-latest.sh [--check|--apply] [--force]

Keep agent-hermes on the newest tagged hermes-agent release.

  --check  Report whether a newer release exists; change nothing.
  --apply  Pull it, rewrite the pin, recreate the container, verify health,
           and roll the pin back if the new release fails to come up.
  --force  Retry a release that was recorded as failed by an earlier --apply.
EOF
}

mode=check
force=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check) mode=check; shift ;;
    --apply) mode=apply; shift ;;
    --force) force=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'hermes-update-latest: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

log() { printf '%s hermes-update-latest: %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

for cmd in curl jq docker sort awk; do
  command -v "$cmd" >/dev/null 2>&1 || die "missing required command: $cmd"
done
[ -r "$ENV_FILE" ] || die "cannot read $ENV_FILE"
[ -r "$COMPOSE_FILE" ] || die "cannot read $COMPOSE_FILE"

current_ref() {
  awk -v key="$ENV_KEY" 'index($0, key "=") == 1 { sub(/^[^=]*=/, ""); print; exit }' "$ENV_FILE"
}

# Newest release tag, ignoring `latest`/`main` and any other floating label.
latest_tag() {
  local url="$HUB/$REPO/tags?page_size=100&ordering=last_updated" names
  names=""
  while [ -n "$url" ] && [ "$url" != "null" ]; do
    local page
    page="$(curl -fsS --max-time 30 "$url")" || die "Docker Hub tag listing failed"
    names+="$(printf '%s' "$page" | jq -r '.results[].name')"$'\n'
    url="$(printf '%s' "$page" | jq -r '.next // ""')"
  done
  printf '%s' "$names" \
    | grep -E "$TAG_RE" \
    | sed 's/^v//' \
    | sort -V \
    | tail -1 \
    | sed 's/^/v/'
}

tag_digest() {
  curl -fsS --max-time 30 "$HUB/$REPO/tags/$1" | jq -er '.digest'
}

# An image guaranteed to be present locally, used only as a shell for the
# privileged rewrite. Prefer the image the container already runs.
helper_image() {
  docker inspect "$CONTAINER" --format '{{.Image}}' 2>/dev/null && return 0
  printf '%s' "$(current_ref)"
}

# Rewrites the file in place rather than replacing it. A create-and-rename would
# hand the new inode the container's own uid/gid, and .env is group-readable to
# `hostdocker` precisely so the unprivileged side can still read it — losing that
# group breaks `docker compose`, which reads .env as the invoking user.
write_pin() {
  local ref="$1" helper
  helper="$(helper_image)"
  awk -v key="$ENV_KEY" -v val="$ref" '
    index($0, key "=") == 1 { print key "=" val; found = 1; next }
    { print }
    END { if (!found) print key "=" val }
  ' "$ENV_FILE" \
    | docker run -i --rm -v "$STACK_DIR:/work" --entrypoint sh "$helper" -c '
        set -e
        cat > /tmp/env.new
        grep -q "^'"$ENV_KEY"'=" /tmp/env.new
        cat /tmp/env.new > /work/.env
      ' \
    || die "failed to rewrite $ENV_KEY in $ENV_FILE"

  [ "$(current_ref)" = "$ref" ] || die "$ENV_FILE did not take the new pin"
}

recreate() {
  docker compose --project-directory "$STACK_DIR" -p "$PROJECT" -f "$COMPOSE_FILE" \
    up -d "$SERVICE" >/dev/null
}

await_health() {
  local deadline=$(( SECONDS + HEALTH_TIMEOUT )) status
  while [ "$SECONDS" -lt "$deadline" ]; do
    status="$(docker inspect -f '{{.State.Health.Status}}' "$CONTAINER" 2>/dev/null || printf 'missing')"
    case "$status" in
      healthy) return 0 ;;
      unhealthy) return 1 ;;
    esac
    sleep 5
  done
  return 1
}

# Health only proves the HTTP surface is up; this proves the agent can still
# resolve its provider and model, which is what a bad release tends to break.
# The CLI entrypoint lags the healthcheck by a few seconds on a cold container,
# so a single shot here reports a false failure and triggers a needless rollback.
smoke() {
  local deadline=$(( SECONDS + SMOKE_TIMEOUT )) out
  while :; do
    # Capture, then match in-shell. Piping into `grep -q` lets grep close the
    # pipe on the first match, which SIGPIPEs the CLI mid-write; under
    # `pipefail` that turns a passing check into a failure, and it only shows up
    # once the status output grows past one pipe buffer.
    out="$(docker exec "$CONTAINER" /opt/hermes/bin/hermes status 2>/dev/null || true)"
    if [[ $out =~ (^|$'\n')[[:space:]]*Model:[[:space:]]+[^[:space:]] ]]; then
      return 0
    fi
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 5
  done
}

mkdir -p "$STATE_DIR"
blocked_file="$STATE_DIR/blocked"

current="$(current_ref)"
[ -n "$current" ] || die "$ENV_KEY not set in $ENV_FILE"

tag="$(latest_tag)"
[ -n "$tag" ] || die "no release tag matched $TAG_RE"
digest="$(tag_digest "$tag")" || die "could not resolve digest for $tag"
desired="$REPO:$tag@$digest"

if [ "$current" = "$desired" ]; then
  log "already on the newest release ($tag)"
  exit 0
fi

if [ "$force" -eq 0 ] && [ -f "$blocked_file" ] && [ "$(cat "$blocked_file")" = "$desired" ]; then
  log "skipping $tag: a previous --apply failed to bring it up; re-run with --force to retry"
  exit 0
fi

log "current:  $current"
log "newest:   $desired"

if [ "$mode" = check ]; then
  log "update available ($tag) — run with --apply to install it"
  exit 0
fi

log "pulling $desired"
docker pull --quiet "$desired" >/dev/null || die "pull failed; leaving $ENV_KEY untouched"

log "rewriting $ENV_KEY and recreating $CONTAINER"
write_pin "$desired"

rollback() {
  log "rolling back to $current"
  write_pin "$current"
  if recreate && await_health; then
    log "rollback complete; $CONTAINER is healthy on the previous release"
  else
    log "ROLLBACK FAILED: $CONTAINER is not healthy on $current — manual recovery required" >&2
  fi
  printf '%s\n' "$desired" > "$blocked_file"
  log "recorded $tag as blocked; re-run with --force to retry it"
}

if ! recreate; then
  rollback
  die "compose could not recreate $SERVICE on $tag"
fi

if ! await_health; then
  rollback
  die "$CONTAINER did not become healthy on $tag within ${HEALTH_TIMEOUT}s"
fi

if ! smoke; then
  rollback
  die "$CONTAINER is healthy on $tag but 'hermes status' reports no model"
fi

rm -f "$blocked_file"
printf '%s\t%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$desired" >> "$STATE_DIR/applied.log"
log "updated to $tag and verified healthy"
