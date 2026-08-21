#!/usr/bin/env bash
# Report n8n automation that has gone quiet, not just automation that errored.
#
# The Gmail labelling workflow sat dead for 90 days on an expired OAuth token.
# That failure happens inside the polling trigger, which never produces an
# execution record, so an n8n error workflow would not have fired once. What is
# observable is the absence of successful executions and the credential error
# n8n logs on every poll, so this checks both and delivers through Hermes,
# which already holds the Discord credentials.
set -Eeuo pipefail

readonly N8N_URL=http://127.0.0.1:5678
readonly N8N_CONTAINER=agent-n8n
readonly HERMES_CONTAINER=agent-hermes
readonly ENV_FILE=/opt/agent-apps/secrets/app.env

# #자동화-오류
DISCORD_TARGET="${N8N_WATCH_DISCORD:-discord:1505473716920909915}"
STALE_HOURS="${N8N_WATCH_STALE_HOURS:-24}"
LOG_WINDOW="${N8N_WATCH_LOG_WINDOW:-30m}"
COOLDOWN_HOURS="${N8N_WATCH_COOLDOWN_HOURS:-12}"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/n8n-workflow-watch"

mode=check
[ "${1:-}" = "--notify" ] && mode=notify
[ "${1:-}" = "--check" ] && mode=check
case "${1:-}" in
  ""|--check|--notify) ;;
  -h|--help) printf 'Usage: n8n-workflow-watch.sh [--check|--notify]\n'; exit 0 ;;
  *) printf 'n8n-workflow-watch: unknown argument: %s\n' "$1" >&2; exit 2 ;;
esac

log() { printf '%s n8n-workflow-watch: %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

for cmd in curl jq docker date; do
  command -v "$cmd" >/dev/null 2>&1 || die "missing required command: $cmd"
done
[ -r "$ENV_FILE" ] || die "cannot read $ENV_FILE"
api_key="$(awk -F= '/^N8N_API_KEY=/{sub(/^[^=]*=/,""); print; exit}' "$ENV_FILE")"
[ -n "$api_key" ] || die "N8N_API_KEY not set in $ENV_FILE"

api() { curl -fsS -m 20 -H "X-N8N-API-KEY: $api_key" "$N8N_URL$1"; }

problems=()

# --- Is n8n itself up? -------------------------------------------------------
health="$(docker inspect -f '{{.State.Health.Status}}' "$N8N_CONTAINER" 2>/dev/null || printf 'missing')"
if [ "$health" != "healthy" ]; then
  problems+=("n8n container is '$health'")
fi
if ! api /api/v1/workflows >/dev/null 2>&1; then
  problems+=("n8n API is not answering on $N8N_URL")
  # Without the API there is nothing further to inspect.
  workflows=""
else
  workflows="$(api '/api/v1/workflows?active=true' | jq -c '.data[] | {id,name}')"
  [ -n "$workflows" ] || problems+=("no active workflows — every automation is switched off")
fi

# --- Has each active workflow actually run? ---------------------------------
now_epoch="$(date -u +%s)"
stale_secs=$(( STALE_HOURS * 3600 ))

while IFS= read -r wf; do
  [ -n "$wf" ] || continue
  id="$(printf '%s' "$wf" | jq -r .id)"
  name="$(printf '%s' "$wf" | jq -r .name)"

  execs="$(api "/api/v1/executions?workflowId=$id&limit=20" || printf '{"data":[]}')"
  last_started="$(printf '%s' "$execs" | jq -r '[.data[].startedAt] | map(select(. != null)) | sort | last // ""')"

  if [ -z "$last_started" ]; then
    problems+=("'$name' has never executed — the trigger is not firing")
  else
    last_epoch="$(date -u -d "$last_started" +%s 2>/dev/null || printf 0)"
    age=$(( now_epoch - last_epoch ))
    if [ "$last_epoch" -gt 0 ] && [ "$age" -gt "$stale_secs" ]; then
      problems+=("'$name' last ran $(( age / 3600 ))h ago (threshold ${STALE_HOURS}h)")
    fi
  fi

  failed="$(printf '%s' "$execs" | jq '[.data[] | select(.status == "error" or .status == "crashed")] | length')"
  [ "$failed" -gt 0 ] && problems+=("'$name' has $failed failed execution(s) in the last 20")
done <<< "$workflows"

# --- What is n8n complaining about in its own log? --------------------------
# Trigger-level failures never reach the execution table, so the log is the
# only place they surface.
trigger_errs="$(docker logs "$N8N_CONTAINER" --since "$LOG_WINDOW" 2>&1 \
  | grep -oE "There was a problem in '[^']+' node in workflow '[^']+'.*" \
  | sed -E "s/(revoked access|refresh token expired).*/\1 — reconnect the credential in n8n./" \
  | sort -u | head -5 || true)"
if [ -n "$trigger_errs" ]; then
  while IFS= read -r line; do
    [ -n "$line" ] && problems+=("$line")
  done <<< "$trigger_errs"
fi

# --- Report ------------------------------------------------------------------
if [ "${#problems[@]}" -eq 0 ]; then
  log "all active workflows healthy"
  exit 0
fi

body="$(printf '%s\n' "${problems[@]}" | sed 's/^/• /')"
log "found ${#problems[@]} problem(s):"
printf '%s\n' "$body"

[ "$mode" = notify ] || exit 0

mkdir -p "$STATE_DIR"
fingerprint="$(printf '%s' "$body" | sha256sum | cut -d' ' -f1)"
stamp_file="$STATE_DIR/last-alert"
if [ -f "$stamp_file" ]; then
  read -r prev_fp prev_epoch < "$stamp_file" || true
  if [ "${prev_fp:-}" = "$fingerprint" ] \
     && [ $(( now_epoch - ${prev_epoch:-0} )) -lt $(( COOLDOWN_HOURS * 3600 )) ]; then
    log "same problems as the last alert and inside the ${COOLDOWN_HOURS}h cooldown — staying quiet"
    exit 0
  fi
fi

if printf '%s' "$body" | docker exec -i "$HERMES_CONTAINER" \
    /opt/hermes/bin/hermes send -q -t "$DISCORD_TARGET" \
    -s "n8n 자동화 점검 실패 ($(date '+%Y-%m-%d %H:%M %Z'))" -f - ; then
  printf '%s %s\n' "$fingerprint" "$now_epoch" > "$stamp_file"
  log "alert delivered to $DISCORD_TARGET"
else
  die "could not deliver the alert through Hermes"
fi
