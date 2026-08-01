#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: update-latest.sh [--check|--apply] [--force]

Keep the Orca headless runtime on the latest stable upstream release.

  --check  Report whether an update is available without changing the runtime.
  --apply  Download, verify, activate, and smoke-test an available update.
  --force  Retry a release that was blocked after a failed activation.
EOF
}

mode=apply
force=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check)
      mode=check
      shift
      ;;
    --apply)
      mode=apply
      shift
      ;;
    --force)
      force=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'update-latest.sh: unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for command_name in \
  base64 curl file flock git grep jq node od pgrep sha256sum sha512sum \
  sort stat systemctl tar tr; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'update-latest.sh: required command is missing: %s\n' \
      "$command_name" >&2
    exit 1
  fi
done

if [ "$(uname -m)" != x86_64 ]; then
  printf 'update-latest.sh: latest channel supports x86_64, not %s\n' \
    "$(uname -m)" >&2
  exit 1
fi

asset=orca-linux.AppImage
latest_metadata_asset=latest-linux.yml
api_url=https://api.github.com/repos/stablyai/orca/releases/latest
install_root=$HOME/.local/orca
release_root=$install_root/releases
current_link=$install_root/current
install_lock=$install_root/.install.lock
default_project_path=$HOME/Documents/Programming/home-server-infra

if ! systemctl --user show-environment >/dev/null; then
  printf '%s\n' \
    'update-latest.sh: cannot read the user manager environment' >&2
  exit 1
fi
service_xdg_state_home=$(
  while IFS= read -r environment_line; do
    case "$environment_line" in
      XDG_STATE_HOME=*)
        printf '%s' "${environment_line#XDG_STATE_HOME=}"
        break
        ;;
    esac
  done < <(systemctl --user show-environment)
)
state_root=${service_xdg_state_home:-$HOME/.local/state}
if [[ "$state_root" != /* ]]; then
  printf '%s\n' \
    'update-latest.sh: user manager state root must be absolute' >&2
  exit 1
fi
state_dir=$state_root/orca-home
readiness=$state_dir/serve-ready.json
rollback_root=$state_dir/update-rollbacks
update_state=$state_dir/update-state.json
blocked_state=$state_dir/update-blocked.json

install -d -m 0755 -- "$install_root" "$release_root"
install -d -m 0700 -- "$state_dir" "$rollback_root"
original_umask=$(umask)
umask 0077
: >>"$install_lock"
umask "$original_umask"
chmod 0600 -- "$install_lock"
exec 9<>"$install_lock"
flock --exclusive 9
umask 0077

metadata_dir=
staging_dir=
link_staging=$install_root/.current.update.$$
state_staging=
old_target=
new_target=
latest_tag=
latest_version=
rollback_dir=
service_stopped=0
activation_started=0
switched=0

cleanup() {
  if [ -n "$rollback_dir" ] \
    && { [ -e "$rollback_dir/.incomplete" ] \
      || [ -L "$rollback_dir/.incomplete" ]; }; then
    if ! remove_incomplete_rollback "$rollback_dir"; then
      printf 'update-latest.sh: preserving untrusted incomplete rollback path: %s\n' \
        "$rollback_dir" >&2
    fi
  fi
  if [ -n "$metadata_dir" ]; then
    case "$metadata_dir" in
      "$state_dir"/.update-check.*)
        /usr/bin/rm -rf --one-file-system -- "$metadata_dir"
        ;;
      *)
        printf 'update-latest.sh: refusing unexpected metadata cleanup: %s\n' \
          "$metadata_dir" >&2
        ;;
    esac
  fi
  if [ -n "$staging_dir" ]; then
    case "$staging_dir" in
      "$release_root"/."$latest_tag".install.*)
        /usr/bin/rm -rf --one-file-system -- "$staging_dir"
        ;;
      *)
        printf 'update-latest.sh: refusing unexpected release cleanup: %s\n' \
          "$staging_dir" >&2
        ;;
    esac
  fi
  if [ -n "$state_staging" ] \
    && { [ -e "$state_staging" ] || [ -L "$state_staging" ]; }; then
    case "$state_staging" in
      "$state_dir"/.*.tmp.*)
        /usr/bin/unlink -- "$state_staging"
        ;;
      *)
        printf 'update-latest.sh: refusing unexpected state cleanup: %s\n' \
          "$state_staging" >&2
        ;;
    esac
  fi
  if [ -e "$link_staging" ] || [ -L "$link_staging" ]; then
    /usr/bin/unlink -- "$link_staging"
  fi
}

write_blocked_state() {
  local reason=$1
  local now

  if [ -z "$latest_tag" ]; then
    return 0
  fi
  now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  state_staging=$state_dir/.update-blocked.tmp.$$
  jq -n \
    --arg tag "$latest_tag" \
    --arg version "$latest_version" \
    --arg failed_at "$now" \
    --arg reason "$reason" \
    '{
      schemaVersion: 1,
      tag: $tag,
      version: $version,
      failedAt: $failed_at,
      reason: $reason
    }' >"$state_staging"
  chmod 0600 -- "$state_staging"
  mv -T -- "$state_staging" "$blocked_state"
  state_staging=
}

switch_current() {
  local target=$1

  if [ -e "$link_staging" ] || [ -L "$link_staging" ]; then
    /usr/bin/unlink -- "$link_staging"
  fi
  ln -s -- "$target" "$link_staging"
  mv -T -- "$link_staging" "$current_link"
}

wait_for_ready() {
  local ready=0

  for _ in $(seq 1 60); do
    if systemctl --user is-active --quiet orca-serve.service \
      && [ -s "$readiness" ] \
      && jq -e '
        type == "object" and
        .type == "orca_server_ready" and
        .schemaVersion == 1 and
        .endpoint == "ws://0.0.0.0:6768" and
        .boundEndpoint == "ws://0.0.0.0:6768" and
        .advertisedEndpoint == "wss://orca.dongwontuna.net" and
        .managedWslCliReconciliation == "settled" and
        .pairing.available == true and
        .pairing.endpoint == "wss://orca.dongwontuna.net" and
        .pairing.scope == "runtime"
      ' "$readiness" >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 1
  done
  if [ "$ready" -ne 1 ]; then
    printf '%s\n' \
      'update-latest.sh: Orca did not produce the v1 readiness contract' >&2
    return 1
  fi
  if [ "$(stat -c '%a' -- "$readiness")" != 600 ] \
    || [ "$(stat -c '%a' -- "$state_dir")" != 700 ]; then
    printf '%s\n' \
      'update-latest.sh: readiness permissions are not private' >&2
    return 1
  fi
}

calculate_tree_sha256() {
  local root=$1
  local digest_output

  digest_output=$(
    tar \
      --sort=name \
      --format=gnu \
      --mtime=@0 \
      --owner=0 \
      --group=0 \
      --numeric-owner \
      --hard-dereference \
      -C "$root" \
      -cf - \
      squashfs-root \
      | sha256sum
  )
  printf '%s' "${digest_output%% *}"
}

verify_dynamic_release() {
  local root=$1
  local layout=${2:-final}
  local metadata=$root/release.json
  local root_name
  local version
  local tag
  local expected_size
  local expected_sha256
  local expected_tree_sha256
  local actual_size
  local actual_sha256
  local actual_tree_sha256
  local file_info

  if [ ! -d "$root" ] || [ -L "$root" ] \
    || [ ! -f "$metadata" ] || [ -L "$metadata" ]; then
    return 1
  fi
  if ! jq -e '
    .schema_version == "orca-home.dynamic-release.v1" and
    .channel == "latest" and
    (.version | type == "string" and test("^[0-9]+[.][0-9]+[.][0-9]+$")) and
    (.tag | type == "string") and
    .asset == "orca-linux.AppImage" and
    (.url | type == "string") and
    (.size | type == "number" and . > 0 and floor == .) and
    (.sha256 | type == "string" and test("^[0-9a-f]{64}$")) and
    (.sha512 | type == "string" and test("^[A-Za-z0-9+/]+={0,2}$")) and
    (.extracted_tree_sha256 | type == "string" and test("^[0-9a-f]{64}$")) and
    (.github_release_id | type == "number" and . > 0 and floor == .) and
    (.published_at | type == "string") and
    (.installed_at | type == "string")
  ' "$metadata" >/dev/null; then
    return 1
  fi

  version=$(jq -er .version "$metadata")
  tag=$(jq -er .tag "$metadata")
  expected_size=$(jq -er '.size | tostring' "$metadata")
  expected_sha256=$(jq -er .sha256 "$metadata")
  expected_tree_sha256=$(jq -er .extracted_tree_sha256 "$metadata")
  if [ "$tag" != "v$version" ] \
    || [ "$(jq -er .url "$metadata")" \
      != "https://github.com/stablyai/orca/releases/download/$tag/$asset" ]; then
    return 1
  fi
  root_name=${root##*/}
  case "$layout" in
    final)
      if [ "$root" != "$release_root/$tag" ]; then
        return 1
      fi
      ;;
    staging)
      if [ "$root" != "$release_root/$root_name" ] \
        || [[ "$root_name" != ".$tag.install."* ]]; then
        return 1
      fi
      ;;
    *)
      return 1
      ;;
  esac
  if [ ! -f "$root/$asset" ] || [ -L "$root/$asset" ] \
    || [ ! -x "$root/squashfs-root/AppRun" ]; then
    return 1
  fi

  actual_size=$(stat -c '%s' -- "$root/$asset")
  actual_sha256=$(sha256sum -- "$root/$asset")
  actual_sha256=${actual_sha256%% *}
  if [ "$actual_size" != "$expected_size" ] \
    || [ "$actual_sha256" != "$expected_sha256" ]; then
    return 1
  fi
  file_info=$(LC_ALL=C file -- "$root/$asset")
  if ! grep -Eq 'ELF .* executable' <<<"$file_info" \
    || ! grep -Fq 'x86-64' <<<"$file_info"; then
    return 1
  fi
  actual_tree_sha256=$(calculate_tree_sha256 "$root")
  [ "$actual_tree_sha256" = "$expected_tree_sha256" ]
}

is_private_owned_directory() {
  local path=$1

  [ -d "$path" ] \
    && [ ! -L "$path" ] \
    && [ "$(stat -c '%u' -- "$path")" = "$(id -u)" ] \
    && [ "$(stat -c '%a' -- "$path")" = 700 ]
}

is_private_owned_file() {
  local path=$1

  [ -f "$path" ] \
    && [ ! -L "$path" ] \
    && [ "$(stat -c '%u' -- "$path")" = "$(id -u)" ] \
    && [ "$(stat -c '%a' -- "$path")" = 600 ]
}

validate_incomplete_marker() {
  local directory=$1
  local name=${directory##*/}
  local old_tag
  local new_tag
  local marker=$directory/.incomplete

  if [ "$directory" != "$rollback_root/$name" ] \
    || [[ ! "$name" =~ ^rollback-(v[0-9]+[.][0-9]+[.][0-9]+)-to-(v[0-9]+[.][0-9]+[.][0-9]+)-[0-9]{8}T[0-9]{6}Z$ ]]; then
    return 1
  fi
  old_tag=${BASH_REMATCH[1]}
  new_tag=${BASH_REMATCH[2]}
  if ! is_private_owned_directory "$directory" \
    || ! is_private_owned_file "$marker"; then
    return 1
  fi
  jq -e \
    --arg directory "$name" \
    --arg old_target "releases/$old_tag" \
    --arg new_target "releases/$new_tag" \
    '
      type == "object" and
      .schemaVersion == 1 and
      .kind == "orca-home.rollback.incomplete" and
      .directory == $directory and
      .oldTarget == $old_target and
      .newTarget == $new_target and
      (.createdAt | type == "string" and
        test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    ' "$marker" >/dev/null
}

validate_rollback_payload() {
  local directory=$1
  local marker_policy=$2
  local name=${directory##*/}
  local old_tag
  local new_tag
  local manifest=$directory/manifest.json
  local archive=$directory/profiles.tar
  local manifest_schema
  local expected_size
  local expected_sha256
  local actual_sha256
  local lower_expected=0
  local upper_expected=0
  local lower_seen=0
  local upper_seen=0
  local entry
  local member
  local entries=()

  if [ "$directory" != "$rollback_root/$name" ] \
    || [[ ! "$name" =~ ^rollback-(v[0-9]+[.][0-9]+[.][0-9]+)-to-(v[0-9]+[.][0-9]+[.][0-9]+)-[0-9]{8}T[0-9]{6}Z$ ]]; then
    return 1
  fi
  old_tag=${BASH_REMATCH[1]}
  new_tag=${BASH_REMATCH[2]}
  if ! is_private_owned_directory "$directory" \
    || ! is_private_owned_file "$manifest" \
    || ! is_private_owned_file "$archive"; then
    return 1
  fi

  shopt -s nullglob dotglob
  entries=("$directory"/*)
  shopt -u nullglob dotglob
  for entry in "${entries[@]}"; do
    case "${entry##*/}" in
      manifest.json|profiles.tar)
        if ! is_private_owned_file "$entry"; then
          return 1
        fi
        ;;
      .incomplete)
        if [ "$marker_policy" != required ] \
          || ! validate_incomplete_marker "$directory"; then
          return 1
        fi
        ;;
      *)
        return 1
        ;;
    esac
  done
  case "$marker_policy" in
    required)
      if ! validate_incomplete_marker "$directory" \
        || [ "${#entries[@]}" -ne 3 ]; then
        return 1
      fi
      ;;
    forbidden)
      if [ -e "$directory/.incomplete" ] \
        || [ -L "$directory/.incomplete" ] \
        || [ "${#entries[@]}" -ne 2 ]; then
        return 1
      fi
      ;;
    *)
      return 1
      ;;
  esac

  if ! jq -e \
    --arg old_target "releases/$old_tag" \
    --arg new_target "releases/$new_tag" \
    '
      type == "object" and
      (.schemaVersion == 1 or .schemaVersion == 2) and
      .oldTarget == $old_target and
      .newTarget == $new_target and
      (.createdAt | type == "string" and
        test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")) and
      (.profiles | type == "array") and
      ([.profiles[] |
        select(. != ".config/orca" and . != ".config/Orca")] | length == 0) and
      ((.profiles | length) == (.profiles | unique | length)) and
      if .schemaVersion == 2 then
        (.profilesTarSize | type == "number" and . > 0 and floor == .) and
        (.profilesTarSha256 | type == "string" and test("^[0-9a-f]{64}$"))
      else
        true
      end
    ' "$manifest" >/dev/null; then
    return 1
  fi

  manifest_schema=$(jq -er '.schemaVersion | tostring' "$manifest")
  if [ "$manifest_schema" = 2 ]; then
    expected_size=$(jq -er '.profilesTarSize | tostring' "$manifest")
    expected_sha256=$(jq -er .profilesTarSha256 "$manifest")
    actual_sha256=$(sha256sum -- "$archive")
    actual_sha256=${actual_sha256%% *}
    if [ "$(stat -c '%s' -- "$archive")" != "$expected_size" ] \
      || [ "$actual_sha256" != "$expected_sha256" ]; then
      return 1
    fi
  fi
  if ! tar -tf "$archive" >/dev/null; then
    return 1
  fi
  if jq -e '.profiles | index(".config/orca") != null' \
    "$manifest" >/dev/null; then
    lower_expected=1
  fi
  if jq -e '.profiles | index(".config/Orca") != null' \
    "$manifest" >/dev/null; then
    upper_expected=1
  fi
  while IFS= read -r member; do
    case "$member" in
      .config/orca|.config/orca/*)
        if [ "$lower_expected" -ne 1 ]; then
          return 1
        fi
        lower_seen=1
        ;;
      .config/Orca|.config/Orca/*)
        if [ "$upper_expected" -ne 1 ]; then
          return 1
        fi
        upper_seen=1
        ;;
      *)
        return 1
        ;;
    esac
  done < <(tar -tf "$archive")
  [ "$lower_seen" -eq "$lower_expected" ] \
    && [ "$upper_seen" -eq "$upper_expected" ]
}

validate_rollback_bundle() {
  validate_rollback_payload "$1" forbidden
}

remove_incomplete_rollback() {
  local directory=$1
  local entry
  local entries=()

  if ! validate_incomplete_marker "$directory"; then
    return 1
  fi
  shopt -s nullglob dotglob
  entries=("$directory"/*)
  shopt -u nullglob dotglob
  for entry in "${entries[@]}"; do
    case "${entry##*/}" in
      .incomplete|manifest.json|profiles.tar)
        if ! is_private_owned_file "$entry"; then
          return 1
        fi
        ;;
      *)
        return 1
        ;;
    esac
  done
  /usr/bin/rm -rf --one-file-system -- "$directory"
  printf 'Removed incomplete Orca rollback snapshot %s.\n' \
    "${directory##*/}"
}

prune_incomplete_rollbacks() {
  local candidate
  local candidates=()

  shopt -s nullglob
  candidates=("$rollback_root"/rollback-*)
  shopt -u nullglob
  for candidate in "${candidates[@]}"; do
    if [ -e "$candidate/.incomplete" ] \
      || [ -L "$candidate/.incomplete" ]; then
      if ! remove_incomplete_rollback "$candidate"; then
        printf 'update-latest.sh: preserving untrusted incomplete rollback path: %s\n' \
          "$candidate" >&2
      fi
    fi
  done
}

prune_retained_state() {
  local active_target=$1
  local from_tag
  local to_tag
  local previous_target
  local keep_rollback
  local candidate
  local candidate_name
  local candidates=()

  if [ ! -e "$update_state" ]; then
    return 0
  fi
  if ! is_private_owned_file "$update_state" \
    || ! jq -e '
      type == "object" and
      .schemaVersion == 1 and
      .channel == "latest" and
      (.from | type == "string" and
        test("^v[0-9]+[.][0-9]+[.][0-9]+$")) and
      (.to | type == "string" and
        test("^v[0-9]+[.][0-9]+[.][0-9]+$")) and
      .from != .to and
      (.updatedAt | type == "string") and
      (.rollback | type == "string") and
      .localWebSocket == 101 and
      .publicWebSocket == 101 and
      .canonicalWorktree == true
    ' "$update_state" >/dev/null; then
    printf '%s\n' \
      'update-latest.sh: retention skipped because update-state.json is untrusted' >&2
    return 0
  fi

  from_tag=$(jq -er .from "$update_state")
  to_tag=$(jq -er .to "$update_state")
  previous_target=releases/$from_tag
  keep_rollback=$(jq -er .rollback "$update_state")
  candidate_name=${keep_rollback##*/}
  if [ "$active_target" != "releases/$to_tag" ] \
    || [ "$keep_rollback" != "$rollback_root/$candidate_name" ] \
    || ! is_private_owned_directory "$install_root/$previous_target" \
    || ! validate_rollback_bundle "$keep_rollback" \
    || ! jq -e \
      --arg old_target "$previous_target" \
      --arg new_target "$active_target" \
      '.oldTarget == $old_target and .newTarget == $new_target' \
      "$keep_rollback/manifest.json" >/dev/null; then
    printf '%s\n' \
      'update-latest.sh: retention skipped because the current rollback generation is incomplete' >&2
    return 0
  fi

  shopt -s nullglob
  candidates=("$rollback_root"/rollback-*)
  shopt -u nullglob
  for candidate in "${candidates[@]}"; do
    if [ "$candidate" = "$keep_rollback" ]; then
      continue
    fi
    if validate_rollback_bundle "$candidate"; then
      if /usr/bin/rm -rf --one-file-system -- "$candidate"; then
        printf 'Pruned superseded Orca rollback snapshot %s.\n' \
          "${candidate##*/}"
      else
        printf 'update-latest.sh: could not prune rollback snapshot: %s\n' \
          "$candidate" >&2
      fi
    fi
  done

  candidates=()
  shopt -s nullglob
  candidates=("$release_root"/v*)
  shopt -u nullglob
  for candidate in "${candidates[@]}"; do
    candidate_name=${candidate##*/}
    if [[ ! "$candidate_name" =~ ^v[0-9]+[.][0-9]+[.][0-9]+$ ]] \
      || [ "releases/$candidate_name" = "$active_target" ] \
      || [ "releases/$candidate_name" = "$previous_target" ] \
      || ! is_private_owned_directory "$candidate" \
      || [ ! -f "$candidate/release.json" ] \
      || [ -L "$candidate/release.json" ] \
      || [ "$(stat -c '%u' -- "$candidate/release.json")" != "$(id -u)" ] \
      || ! jq -e \
        '.schema_version == "orca-home.dynamic-release.v1"' \
        "$candidate/release.json" >/dev/null 2>&1; then
      continue
    fi
    if verify_dynamic_release "$candidate"; then
      if /usr/bin/rm -rf --one-file-system -- "$candidate"; then
        printf 'Pruned superseded Orca dynamic release %s.\n' \
          "$candidate_name"
      else
        printf 'update-latest.sh: could not prune dynamic release: %s\n' \
          "$candidate" >&2
      fi
    else
      printf 'update-latest.sh: preserving unverified dynamic release: %s\n' \
        "$candidate" >&2
    fi
  done
}

version_compare() {
  local left=$1
  local right=$2
  local left_major
  local left_minor
  local left_patch
  local right_major
  local right_minor
  local right_patch

  IFS=. read -r left_major left_minor left_patch <<<"$left"
  IFS=. read -r right_major right_minor right_patch <<<"$right"
  for component in major minor patch; do
    local left_name=left_$component
    local right_name=right_$component
    local left_value=$((10#${!left_name}))
    local right_value=$((10#${!right_name}))
    if [ "$left_value" -lt "$right_value" ]; then
      printf '%s' -1
      return 0
    fi
    if [ "$left_value" -gt "$right_value" ]; then
      printf '%s' 1
      return 0
    fi
  done
  printf '%s' 0
}

probe_websocket() {
  local url=$1
  local code
  local status=0

  code=$(
    curl \
      --http1.1 \
      --silent \
      --show-error \
      --max-time 5 \
      --output /dev/null \
      --write-out '%{http_code}' \
      --header 'Connection: Upgrade' \
      --header 'Upgrade: websocket' \
      --header 'Sec-WebSocket-Version: 13' \
      --header 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
      "$url"
  ) || status=$?
  if [ "$code" != 101 ] || { [ "$status" -ne 0 ] && [ "$status" -ne 28 ]; }; then
    printf 'update-latest.sh: WebSocket probe failed for %s (HTTP %s)\n' \
      "$url" "${code:-none}" >&2
    return 1
  fi
}

verify_default_project() {
  local orca_cli=$current_link/squashfs-root/resources/app.asar.unpacked/out/cli/index.js
  local repo_add_json
  local repo_id
  local worktree_list_json

  if [ ! -f "$orca_cli" ] || [ -L "$orca_cli" ] \
    || [ ! -d "$default_project_path" ] \
    || [ -L "$default_project_path" ] \
    || [ "$(git -C "$default_project_path" rev-parse --show-toplevel 2>/dev/null)" \
      != "$default_project_path" ]; then
    printf '%s\n' \
      'update-latest.sh: canonical project or version-matched CLI is unavailable' >&2
    return 1
  fi

  (
    export ORCA_PAIRING_CODE
    ORCA_PAIRING_CODE=$(jq -er '
      .pairing.url
      | select(type == "string" and startswith("orca://pair?"))
    ' "$readiness")
    repo_add_json=$(
      node "$orca_cli" repo add --path "$default_project_path" --json
    )
    if ! jq -e --arg path "$default_project_path" '
      .ok == true and
      .result.repo.path == $path and
      .result.repo.kind == "git" and
      (.result.repo.id | type == "string" and length > 0)
    ' <<<"$repo_add_json" >/dev/null; then
      return 1
    fi
    repo_id=$(jq -er '.result.repo.id' <<<"$repo_add_json")
    worktree_list_json=$(
      node "$orca_cli" worktree list --repo "id:$repo_id" --json
    )
    jq -e --arg path "$default_project_path" --arg repo_id "$repo_id" '
      .ok == true and
      any(.result.worktrees[]; .path == $path and .repoId == $repo_id)
    ' <<<"$worktree_list_json" >/dev/null
  )
}

assert_no_external_orca() {
  local pid
  local cgroup

  while IFS= read -r pid; do
    [ -n "$pid" ] || continue
    if [ ! -r "/proc/$pid/cgroup" ]; then
      continue
    fi
    cgroup=$(<"/proc/$pid/cgroup")
    if ! grep -Fq '/orca-serve.service' <<<"$cgroup"; then
      printf 'update-latest.sh: external Orca process blocks profile snapshot (PID %s)\n' \
        "$pid" >&2
      return 1
    fi
  done < <(pgrep -u "$(id -u)" -x orca-ide || true)
}

stop_managed_daemon() {
  local daemon_dir=$HOME/.config/orca/daemon
  local pid_file
  local pid
  local expected_entry=$install_root/$old_target/squashfs-root/resources/app.asar.unpacked/out/main/daemon-entry.js
  local expected_version=${old_target#releases/v}
  local record_entry
  local record_version
  local comm
  local cgroup
  local candidate
  local candidate_pid
  local candidates=()
  local active_records=()

  if [ ! -e "$daemon_dir" ]; then
    return 0
  fi
  if [ ! -d "$daemon_dir" ] || [ -L "$daemon_dir" ]; then
    printf 'update-latest.sh: daemon state is not a real directory: %s\n' \
      "$daemon_dir" >&2
    return 1
  fi

  shopt -s nullglob
  candidates=("$daemon_dir"/daemon-v*.pid)
  shopt -u nullglob
  for candidate in "${candidates[@]}"; do
    if [[ ! "$(basename -- "$candidate")" =~ ^daemon-v[0-9]+[.]pid$ ]] \
      || [ ! -f "$candidate" ] \
      || [ -L "$candidate" ] \
      || [ "$(stat -c '%u' -- "$candidate")" != "$(id -u)" ] \
      || [ "$(stat -c '%a' -- "$candidate")" != 600 ]; then
      printf 'update-latest.sh: daemon PID record is not trusted: %s\n' \
        "$candidate" >&2
      return 1
    fi
    if ! jq -e '
      type == "object" and
      (.pid | type == "number" and . > 0 and floor == .) and
      (.startedAtMs | type == "number" and . > 0) and
      (.entryPath | type == "string") and
      (.appVersion | type == "string") and
      (.launchNonce | type == "string" and length > 0)
    ' "$candidate" >/dev/null; then
      printf 'update-latest.sh: daemon PID record is malformed: %s\n' \
        "$candidate" >&2
      return 1
    fi
    candidate_pid=$(jq -er '.pid | tostring' "$candidate")
    if kill -0 "$candidate_pid" 2>/dev/null; then
      active_records+=("$candidate")
    fi
  done
  if [ "${#active_records[@]}" -eq 0 ]; then
    return 0
  fi
  if [ "${#active_records[@]}" -ne 1 ]; then
    printf 'update-latest.sh: expected one active daemon record, found %s\n' \
      "${#active_records[@]}" >&2
    return 1
  fi
  pid_file=${active_records[0]}
  pid=$(jq -er '.pid | tostring' "$pid_file")
  record_entry=$(jq -er .entryPath "$pid_file")
  record_version=$(jq -er .appVersion "$pid_file")
  if [ "$record_entry" != "$expected_entry" ] \
    || [ "$record_version" != "$expected_version" ]; then
    printf '%s\n' \
      'update-latest.sh: daemon PID record does not match the active release' >&2
    return 1
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  if [ ! -r "/proc/$pid/comm" ] || [ ! -r "/proc/$pid/cgroup" ]; then
    printf 'update-latest.sh: cannot inspect managed daemon PID %s\n' \
      "$pid" >&2
    return 1
  fi
  comm=$(<"/proc/$pid/comm")
  cgroup=$(<"/proc/$pid/cgroup")
  if [ "$comm" != orca-ide ]; then
    printf 'update-latest.sh: daemon command mismatch for PID %s (%s)\n' \
      "$pid" "$comm" >&2
    return 1
  fi
  if [[ ! "$cgroup" =~ /app-orca-[0-9]+[.]scope$ ]]; then
    printf 'update-latest.sh: daemon scope mismatch for PID %s (%s)\n' \
      "$pid" "$cgroup" >&2
    return 1
  fi

  kill -TERM "$pid"
  for _ in $(seq 1 30); do
    if ! kill -0 "$pid" 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  printf 'update-latest.sh: managed daemon PID %s did not stop after SIGTERM\n' \
    "$pid" >&2
  return 1
}

create_profile_snapshot() {
  local timestamp
  local created_at
  local profile
  local profiles=()
  local profile_json
  local archive_size
  local archive_sha256
  local marker

  timestamp=$(date -u +%Y%m%dT%H%M%SZ)
  created_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  rollback_dir=$rollback_root/rollback-${old_target##*/}-to-${new_target##*/}-$timestamp
  if [ -e "$rollback_dir" ] || [ -L "$rollback_dir" ]; then
    printf 'update-latest.sh: refusing existing rollback path: %s\n' \
      "$rollback_dir" >&2
    return 1
  fi
  install -d -m 0700 -- "$rollback_dir"
  marker=$rollback_dir/.incomplete
  jq -n \
    --arg directory "${rollback_dir##*/}" \
    --arg old_target "$old_target" \
    --arg new_target "$new_target" \
    --arg created_at "$created_at" \
    '{
      schemaVersion: 1,
      kind: "orca-home.rollback.incomplete",
      directory: $directory,
      oldTarget: $old_target,
      newTarget: $new_target,
      createdAt: $created_at
    }' >"$marker"
  chmod 0600 -- "$marker"

  for profile in .config/orca .config/Orca; do
    if [ -e "$HOME/$profile" ] || [ -L "$HOME/$profile" ]; then
      if [ ! -d "$HOME/$profile" ] || [ -L "$HOME/$profile" ]; then
        printf 'update-latest.sh: profile path is not a real directory: %s\n' \
          "$HOME/$profile" >&2
        return 1
      fi
      profiles+=("$profile")
    fi
  done

  if [ "${#profiles[@]}" -gt 0 ]; then
    tar -C "$HOME" -cpf "$rollback_dir/profiles.tar" -- "${profiles[@]}"
  else
    tar -C "$HOME" -cpf "$rollback_dir/profiles.tar" --files-from /dev/null
  fi
  chmod 0600 -- "$rollback_dir/profiles.tar"
  tar -tf "$rollback_dir/profiles.tar" >/dev/null
  archive_size=$(stat -c '%s' -- "$rollback_dir/profiles.tar")
  archive_sha256=$(sha256sum -- "$rollback_dir/profiles.tar")
  archive_sha256=${archive_sha256%% *}

  profile_json=$(
    if [ "${#profiles[@]}" -gt 0 ]; then
      printf '%s\n' "${profiles[@]}" | jq -R . | jq -s .
    else
      printf '%s\n' '[]'
    fi
  )
  jq -n \
    --arg old_target "$old_target" \
    --arg new_target "$new_target" \
    --arg created_at "$created_at" \
    --argjson profiles "$profile_json" \
    --argjson profiles_tar_size "$archive_size" \
    --arg profiles_tar_sha256 "$archive_sha256" \
    '{
      schemaVersion: 2,
      oldTarget: $old_target,
      newTarget: $new_target,
      createdAt: $created_at,
      profiles: $profiles,
      profilesTarSize: $profiles_tar_size,
      profilesTarSha256: $profiles_tar_sha256
    }' >"$rollback_dir/manifest.json"
  chmod 0600 -- "$rollback_dir/manifest.json"
  validate_rollback_payload "$rollback_dir" required
  /usr/bin/unlink -- "$marker"
}

rollback_activation() {
  local failed_profiles=$rollback_dir/failed-new-profile
  local profile
  local recovery_ok=1

  printf 'update-latest.sh: rolling back failed activation of %s\n' \
    "$latest_tag" >&2
  systemctl --user stop orca-serve.service || recovery_ok=0
  install -d -m 0700 -- "$failed_profiles/.config" || recovery_ok=0
  for profile in .config/orca .config/Orca; do
    if [ -e "$HOME/$profile" ] || [ -L "$HOME/$profile" ]; then
      mv -T -- "$HOME/$profile" "$failed_profiles/$profile" || recovery_ok=0
    fi
  done
  switch_current "$old_target" || recovery_ok=0
  tar -C "$HOME" -xpf "$rollback_dir/profiles.tar" || recovery_ok=0
  : >"$readiness" || recovery_ok=0
  chmod 0600 -- "$readiness" || recovery_ok=0
  systemctl --user start orca-serve.service || recovery_ok=0
  wait_for_ready || recovery_ok=0
  if [ "$recovery_ok" -ne 1 ]; then
    printf '%s\n' \
      'update-latest.sh: automatic rollback did not restore a healthy runtime' >&2
    return 1
  fi
  printf 'update-latest.sh: restored %s after failed update\n' \
    "${old_target##*/}" >&2
}

on_error() {
  local status=$1
  local line=$2
  local rollback_status=0

  trap - ERR HUP INT TERM EXIT
  set +e
  printf 'update-latest.sh: command failed at line %s (status %s)\n' \
    "$line" "$status" >&2
  if [ "$activation_started" -eq 1 ]; then
    write_blocked_state "activation failed at line $line"
  fi
  if [ "$switched" -eq 1 ]; then
    rollback_activation
    rollback_status=$?
  elif [ "$service_stopped" -eq 1 ]; then
    systemctl --user start orca-serve.service
  fi
  cleanup
  if [ "$rollback_status" -ne 0 ]; then
    exit 1
  fi
  exit "$status"
}

trap cleanup EXIT
trap 'on_error $? $LINENO' ERR
trap 'on_error 129 $LINENO' HUP
trap 'on_error 130 $LINENO' INT
trap 'on_error 143 $LINENO' TERM

if [ "$mode" = apply ]; then
  prune_incomplete_rollbacks
fi

metadata_dir=$(mktemp -d "$state_dir/.update-check.XXXXXX")
chmod 0700 -- "$metadata_dir"
release_api=$metadata_dir/release.json
latest_metadata=$metadata_dir/$latest_metadata_asset
curl \
  --proto '=https' \
  --proto-redir '=https' \
  --fail \
  --location \
  --retry 3 \
  --silent \
  --show-error \
  --header 'Accept: application/vnd.github+json' \
  --header 'User-Agent: orca-home-latest-updater/1' \
  --output "$release_api" \
  "$api_url"

if ! jq -e '
  type == "object" and
  .draft == false and
  .prerelease == false and
  (.id | type == "number" and . > 0 and floor == .) and
  (.tag_name | type == "string" and test("^v[0-9]+[.][0-9]+[.][0-9]+$")) and
  (.published_at | type == "string")
' "$release_api" >/dev/null; then
  printf '%s\n' \
    'update-latest.sh: GitHub latest release metadata is invalid' >&2
  false
fi

latest_tag=$(jq -er .tag_name "$release_api")
latest_version=${latest_tag#v}
release_id=$(jq -er '.id | tostring' "$release_api")
published_at=$(jq -er .published_at "$release_api")

asset_json=$(
  jq -cer --arg name "$asset" '
    [.assets[] | select(.name == $name)]
    | if length == 1 then .[0] else error("asset cardinality") end
  ' "$release_api"
)
metadata_asset_json=$(
  jq -cer --arg name "$latest_metadata_asset" '
    [.assets[] | select(.name == $name)]
    | if length == 1 then .[0] else error("metadata asset cardinality") end
  ' "$release_api"
)
asset_url=$(jq -er .browser_download_url <<<"$asset_json")
asset_size=$(jq -er '.size | tostring' <<<"$asset_json")
asset_digest=$(jq -er '.digest' <<<"$asset_json")
metadata_url=$(jq -er .browser_download_url <<<"$metadata_asset_json")
metadata_size=$(jq -er '.size | tostring' <<<"$metadata_asset_json")
metadata_digest=$(jq -er '.digest' <<<"$metadata_asset_json")
expected_asset_url=https://github.com/stablyai/orca/releases/download/$latest_tag/$asset
expected_metadata_url=https://github.com/stablyai/orca/releases/download/$latest_tag/$latest_metadata_asset
if [ "$asset_url" != "$expected_asset_url" ] \
  || [ "$metadata_url" != "$expected_metadata_url" ] \
  || [[ ! "$asset_digest" =~ ^sha256:[0-9a-f]{64}$ ]] \
  || [[ ! "$metadata_digest" =~ ^sha256:[0-9a-f]{64}$ ]] \
  || [[ ! "$asset_size" =~ ^[1-9][0-9]+$ ]] \
  || [[ ! "$metadata_size" =~ ^[1-9][0-9]*$ ]]; then
  printf '%s\n' \
    'update-latest.sh: latest release assets violate the trusted contract' >&2
  false
fi
expected_sha256=${asset_digest#sha256:}
expected_metadata_sha256=${metadata_digest#sha256:}

curl \
  --proto '=https' \
  --proto-redir '=https' \
  --fail \
  --location \
  --retry 3 \
  --silent \
  --show-error \
  --output "$latest_metadata" \
  "$metadata_url"
if [ "$(stat -c '%s' -- "$latest_metadata")" != "$metadata_size" ] \
  || [ "$(sha256sum -- "$latest_metadata" | awk '{print $1}')" \
    != "$expected_metadata_sha256" ]; then
  printf '%s\n' \
    'update-latest.sh: latest-linux.yml failed GitHub digest verification' >&2
  false
fi
yaml_version=$(sed -n 's/^version: //p' "$latest_metadata")
yaml_path=$(sed -n 's/^path: //p' "$latest_metadata")
yaml_sha512=$(sed -n 's/^sha512: //p' "$latest_metadata")
if [ "$yaml_version" != "$latest_version" ] \
  || [ "$yaml_path" != "$asset" ] \
  || [[ ! "$yaml_sha512" =~ ^[A-Za-z0-9+/]+={0,2}$ ]] \
  || ! grep -Fqx "    size: $asset_size" "$latest_metadata"; then
  printf '%s\n' \
    'update-latest.sh: latest-linux.yml does not match the GitHub release' >&2
  false
fi

if [ ! -L "$current_link" ]; then
  printf '%s\n' \
    'update-latest.sh: current runtime selector is not a symlink' >&2
  false
fi
old_target=$(readlink -- "$current_link")
if [[ ! "$old_target" =~ ^releases/v[0-9]+[.][0-9]+[.][0-9]+$ ]] \
  || [ ! -d "$install_root/$old_target" ] \
  || [ -L "$install_root/$old_target" ]; then
  printf 'update-latest.sh: current runtime target is invalid: %s\n' \
    "$old_target" >&2
  false
fi
current_version=${old_target#releases/v}
comparison=$(version_compare "$current_version" "$latest_version")
if [ "$mode" = check ]; then
  if [ "$comparison" -lt 0 ]; then
    update_available=yes
  else
    update_available=no
  fi
  printf 'Orca current=v%s latest=%s update_available=%s\n' \
    "$current_version" "$latest_tag" "$update_available"
  exit 0
fi
prune_retained_state "$old_target"
if [ "$comparison" -gt 0 ]; then
  printf 'update-latest.sh: refusing latest-channel downgrade v%s -> %s\n' \
    "$current_version" "$latest_tag" >&2
  false
fi
if [ "$comparison" -eq 0 ]; then
  printf 'Orca %s is already the latest stable release.\n' "$latest_tag"
  exit 0
fi
if [ "$force" -ne 1 ] \
  && [ -f "$blocked_state" ] \
  && jq -e --arg tag "$latest_tag" \
    '.schemaVersion == 1 and .tag == $tag' "$blocked_state" >/dev/null 2>&1; then
  printf 'update-latest.sh: %s is blocked after a failed activation; waiting for a newer release\n' \
    "$latest_tag" >&2
  exit 0
fi
if ! systemctl --user is-active --quiet orca-serve.service; then
  printf '%s\n' \
    'update-latest.sh: orca-serve.service is inactive; preserving the operator stop' >&2
  exit 0
fi

new_target=releases/$latest_tag
new_release_dir=$install_root/$new_target
if [ -e "$new_release_dir" ] || [ -L "$new_release_dir" ]; then
  if ! verify_dynamic_release "$new_release_dir"; then
    printf 'update-latest.sh: existing candidate is invalid: %s\n' \
      "$new_release_dir" >&2
    false
  fi
else
  staging_dir=$(mktemp -d "$release_root/.${latest_tag}.install.XXXXXX")
  chmod 0700 -- "$staging_dir"
  candidate=$staging_dir/$asset
  curl \
    --proto '=https' \
    --proto-redir '=https' \
    --fail \
    --location \
    --retry 3 \
    --silent \
    --show-error \
    --output "$candidate" \
    "$asset_url"
  chmod 0755 -- "$candidate"
  actual_size=$(stat -c '%s' -- "$candidate")
  actual_sha256=$(sha256sum -- "$candidate")
  actual_sha256=${actual_sha256%% *}
  actual_sha512=$(sha512sum -- "$candidate")
  actual_sha512=${actual_sha512%% *}
  expected_sha512=$(
    printf '%s' "$yaml_sha512" \
      | base64 --decode \
      | od -An -v -tx1 \
      | tr -d ' \n'
  )
  file_info=$(LC_ALL=C file -- "$candidate")
  if [ "$actual_size" != "$asset_size" ] \
    || [ "$actual_sha256" != "$expected_sha256" ] \
    || [ "$actual_sha512" != "$expected_sha512" ] \
    || ! grep -Eq 'ELF .* executable' <<<"$file_info" \
    || ! grep -Fq 'x86-64' <<<"$file_info"; then
    printf '%s\n' \
      'update-latest.sh: downloaded AppImage failed integrity verification' >&2
    false
  fi
  (
    cd "$staging_dir"
    "./$asset" --appimage-extract >/dev/null
  )
  if [ ! -x "$staging_dir/squashfs-root/AppRun" ]; then
    printf '%s\n' \
      'update-latest.sh: AppImage extraction did not produce AppRun' >&2
    false
  fi
  extracted_tree_sha256=$(calculate_tree_sha256 "$staging_dir")
  jq -n \
    --arg version "$latest_version" \
    --arg tag "$latest_tag" \
    --arg asset "$asset" \
    --arg url "$asset_url" \
    --argjson size "$asset_size" \
    --arg sha256 "$expected_sha256" \
    --arg sha512 "$yaml_sha512" \
    --arg extracted_tree_sha256 "$extracted_tree_sha256" \
    --argjson github_release_id "$release_id" \
    --arg published_at "$published_at" \
    --arg installed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{
      schema_version: "orca-home.dynamic-release.v1",
      channel: "latest",
      version: $version,
      tag: $tag,
      asset: $asset,
      url: $url,
      size: $size,
      sha256: $sha256,
      sha512: $sha512,
      extracted_tree_sha256: $extracted_tree_sha256,
      github_release_id: $github_release_id,
      published_at: $published_at,
      installed_at: $installed_at
    }' >"$staging_dir/release.json"
  chmod 0644 -- "$staging_dir/release.json"
  if ! verify_dynamic_release "$staging_dir" staging; then
    printf '%s\n' \
      'update-latest.sh: staged release failed deterministic verification' >&2
    false
  fi
  mv -T -- "$staging_dir" "$new_release_dir"
  staging_dir=
fi

systemctl --user stop orca-serve.service
service_stopped=1
stop_managed_daemon
assert_no_external_orca
create_profile_snapshot
activation_started=1
switch_current "$new_target"
switched=1
: >"$readiness"
chmod 0600 -- "$readiness"
systemctl --user start orca-serve.service
service_stopped=0
wait_for_ready
verify_default_project
probe_websocket http://127.0.0.1:6768/
probe_websocket https://orca.dongwontuna.net/

state_staging=$state_dir/.update-state.tmp.$$
jq -n \
  --arg from "${old_target##*/}" \
  --arg to "$latest_tag" \
  --arg updated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg rollback "$rollback_dir" \
  '{
    schemaVersion: 1,
    channel: "latest",
    from: $from,
    to: $to,
    updatedAt: $updated_at,
    rollback: $rollback,
    localWebSocket: 101,
    publicWebSocket: 101,
    canonicalWorktree: true
  }' >"$state_staging"
chmod 0600 -- "$state_staging"
mv -T -- "$state_staging" "$update_state"
state_staging=
if [ -f "$blocked_state" ]; then
  /usr/bin/unlink -- "$blocked_state"
fi
activation_started=0
switched=0
prune_retained_state "$new_target"

printf 'Updated Orca %s -> %s; local/public WebSocket 101 and canonical worktree verified.\n' \
  "${old_target##*/}" "$latest_tag"
