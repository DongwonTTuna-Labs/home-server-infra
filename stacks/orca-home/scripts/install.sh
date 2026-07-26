#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: install.sh [--source PATH] [--activate]

Install the release-pinned Orca headless runtime and user systemd unit.

  --source PATH  Use an already-downloaded AppImage instead of downloading it.
  --activate     Enable and restart orca-serve.service after installation.
EOF
}

source_path=
activate=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --source)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        printf '%s\n' 'install.sh: --source requires a path' >&2
        exit 2
      fi
      source_path=$2
      shift 2
      ;;
    --activate)
      activate=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'install.sh: unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for command_name in bash curl file grep jq sha256sum stat systemctl systemd-analyze; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'install.sh: required command is missing: %s\n' "$command_name" >&2
    exit 1
  fi
done

if [ "$(uname -m)" != x86_64 ]; then
  printf 'install.sh: release pin supports x86_64, not %s\n' "$(uname -m)" >&2
  exit 1
fi

stack_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
release_json=$stack_dir/release.json
service_source=$stack_dir/systemd/orca-serve.service
runner_source=$stack_dir/scripts/run.sh

if ! jq -e '
  .schema_version == "orca-home.release.v1" and
  (.version | type == "string") and
  (.tag | type == "string") and
  .asset == "orca-linux.AppImage" and
  .architecture == "x86_64" and
  (.url | type == "string") and
  (.size | type == "number" and . > 0 and floor == .) and
  (.sha256 | type == "string" and test("^[0-9a-f]{64}$")) and
  (.source_commit | type == "string" and test("^[0-9a-f]{40}$"))
' "$release_json" >/dev/null; then
  printf '%s\n' 'install.sh: release.json is invalid' >&2
  exit 1
fi

version=$(jq -er .version "$release_json")
tag=$(jq -er .tag "$release_json")
asset=$(jq -er .asset "$release_json")
url=$(jq -er .url "$release_json")
expected_size=$(jq -er '.size | tostring' "$release_json")
expected_sha256=$(jq -er .sha256 "$release_json")

if [[ ! "$version" =~ ^[0-9]+[.][0-9]+[.][0-9]+$ ]] \
  || [ "$tag" != "v$version" ] \
  || [ "$url" != "https://github.com/stablyai/orca/releases/download/$tag/$asset" ]; then
  printf '%s\n' 'install.sh: release identity is inconsistent' >&2
  exit 1
fi

verify_asset() {
  local path=$1
  local actual_size
  local file_info

  if [ ! -f "$path" ] || [ -L "$path" ]; then
    printf 'install.sh: asset is not a regular non-symlink file: %s\n' "$path" >&2
    return 1
  fi
  actual_size=$(stat -c '%s' -- "$path")
  if [ "$actual_size" != "$expected_size" ]; then
    printf 'install.sh: asset size mismatch (expected %s, got %s)\n' \
      "$expected_size" "$actual_size" >&2
    return 1
  fi
  if ! printf '%s  %s\n' "$expected_sha256" "$path" \
    | sha256sum --check --status; then
    printf '%s\n' 'install.sh: asset SHA-256 mismatch' >&2
    return 1
  fi
  file_info=$(LC_ALL=C file -- "$path")
  if ! grep -Eq 'ELF .* executable' <<<"$file_info" \
    || ! grep -Fq 'x86-64' <<<"$file_info"; then
    printf '%s\n' 'install.sh: asset architecture is not x86-64 ELF' >&2
    return 1
  fi
}

if [ -n "$source_path" ]; then
  if [ ! -f "$source_path" ]; then
    printf 'install.sh: source is not a regular file: %s\n' "$source_path" >&2
    exit 1
  fi
  source_path=$(readlink -f -- "$source_path")
  verify_asset "$source_path"
fi

install_root=$HOME/.local/orca
release_root=$install_root/releases
release_dir=$release_root/$tag
current_link=$install_root/current
unit_dir=$HOME/.config/systemd/user
libexec_dir=$HOME/.local/libexec
install -d -m 0755 -- "$install_root" "$release_root" "$unit_dir" "$libexec_dir"

staging_dir=
link_staging=$install_root/.current.$$
cleanup() {
  if [ -n "$staging_dir" ]; then
    case "$staging_dir" in
      "$release_root"/."$tag".install.*)
        /usr/bin/rm -rf -- "$staging_dir"
        ;;
      *)
        printf 'install.sh: refusing unexpected cleanup path: %s\n' "$staging_dir" >&2
        ;;
    esac
  fi
  if [ -e "$link_staging" ] || [ -L "$link_staging" ]; then
    /usr/bin/unlink -- "$link_staging"
  fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

release_ready=0
if [ -e "$release_dir" ] || [ -L "$release_dir" ]; then
  if [ -d "$release_dir" ] \
    && [ ! -L "$release_dir" ] \
    && [ -x "$release_dir/squashfs-root/AppRun" ] \
    && [ -f "$release_dir/release.json" ] \
    && cmp -s -- "$release_json" "$release_dir/release.json" \
    && verify_asset "$release_dir/$asset"; then
    release_ready=1
  else
    printf 'install.sh: existing release directory is incomplete: %s\n' \
      "$release_dir" >&2
    exit 1
  fi
fi

if [ "$release_ready" -eq 0 ]; then
  staging_dir=$(mktemp -d "$release_root/.${tag}.install.XXXXXX")
  candidate=$staging_dir/$asset

  if [ -n "$source_path" ]; then
    cp -- "$source_path" "$candidate"
  else
    curl \
      --proto '=https' \
      --proto-redir '=https' \
      --fail \
      --location \
      --retry 3 \
      --output "$candidate" \
      "$url"
  fi
  chmod 0755 "$candidate"
  verify_asset "$candidate"

  (
    cd "$staging_dir"
    "./$asset" --appimage-extract >/dev/null
  )
  app_run=$staging_dir/squashfs-root/AppRun
  if [ ! -x "$app_run" ]; then
    printf '%s\n' 'install.sh: AppImage extraction did not produce AppRun' >&2
    exit 1
  fi
  install -m 0644 -- "$release_json" "$staging_dir/release.json"
  mv -- "$staging_dir" "$release_dir"
  staging_dir=
fi

target=releases/$tag
if [ -L "$current_link" ]; then
  current_target=$(readlink -- "$current_link")
  if [ "$current_target" != "$target" ]; then
    printf '%s\n' \
      'install.sh: refusing a cross-version switch without an Orca profile rollback bundle' >&2
    exit 1
  fi
elif [ -e "$current_link" ]; then
  printf 'install.sh: current path is not a symlink: %s\n' "$current_link" >&2
  exit 1
else
  ln -s -- "$target" "$link_staging"
  mv -T -- "$link_staging" "$current_link"
fi

bash -n "$runner_source"
systemd-analyze --user verify "$service_source"
install -m 0755 -- "$runner_source" "$libexec_dir/orca-home-run"
install -m 0644 -- "$service_source" "$unit_dir/orca-serve.service"
systemctl --user daemon-reload

if [ "$activate" -eq 1 ]; then
  systemctl --user enable orca-serve.service
  systemctl --user restart orca-serve.service

  readiness=$HOME/.local/state/orca-home/serve-ready.json
  ready=0
  for _ in $(seq 1 45); do
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
        .pairing.scope == "mobile"
      ' "$readiness" >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 1
  done
  if [ "$ready" -ne 1 ]; then
    printf '%s\n' 'install.sh: Orca did not produce the v1 readiness contract' >&2
    exit 1
  fi
  if [ "$(stat -c '%a' -- "$readiness")" != 600 ]; then
    printf '%s\n' 'install.sh: readiness state must have mode 0600' >&2
    exit 1
  fi
fi

printf 'Installed Orca %s AppRun at %s\n' \
  "$version" "$release_dir/squashfs-root/AppRun"
if [ "$activate" -eq 1 ]; then
  printf '%s\n' 'orca-serve.service is enabled and active.'
fi
