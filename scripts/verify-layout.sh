#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

required=(
  README.md
  docs/restore.md
  docs/secrets.md
  stacks/codex-lb/README.md
  stacks/codex-lb/compose.yaml
  stacks/codexpro-home/README.md
  stacks/codexpro-home/cloudflared/codexpro-home.yml
  stacks/codexpro-home/scripts/codexpro-home-url.mjs
  stacks/codexpro-home/systemd/codexpro-home.service
  stacks/codexpro-home/systemd/cloudflared-codexpro-home.service
  stacks/orca-home/README.md
  stacks/orca-home/release.json
  stacks/orca-home/scripts/install.sh
  stacks/orca-home/scripts/run.sh
  stacks/orca-home/scripts/update-latest.sh
  stacks/orca-home/systemd/orca-serve.service
  stacks/orca-home/systemd/orca-update-latest.service
  stacks/orca-home/systemd/orca-update-latest.timer
  scripts/test-credential-scan.sh
  stacks/tunnel-apps/README.md
  stacks/tunnel-apps/compose.yaml
  stacks/tunnel-apps/cloudflared/tunnel-apps.yml
  stacks/coding/README.md
  stacks/coding/systemd/coding-tools.target
  stacks/coding/systemd/codex-cli-update.service
  stacks/coding/systemd/codex-cli-update.timer
  stacks/maintenance/README.md
  stacks/maintenance/compose.yaml
  stacks/codex-github-runners/compose.yaml
  stacks/codex-github-runners/Dockerfile
  stacks/agent-stack/compose.yml
  stacks/agent-stack/secrets/cloudflared.env.example
  dotfiles/codex/config.toml
  dotfiles/codex/rules/default.rules
)

for path in "${required[@]}"; do
  if [ ! -e "$path" ]; then
    printf 'Missing required path: %s\n' "$path" >&2
    exit 1
  fi
done

if ! git ls-files --error-unmatch scripts/test-credential-scan.sh >/dev/null 2>&1; then
  printf 'Credential negative sensor must be tracked: scripts/test-credential-scan.sh\n' >&2
  exit 1
fi

orca_release=stacks/orca-home/release.json
jq -e '
  .schema_version == "orca-home.release.v1" and
  .version == "1.4.156" and
  .tag == "v1.4.156" and
  .asset == "orca-linux.AppImage" and
  .architecture == "x86_64" and
  .url == "https://github.com/stablyai/orca/releases/download/v1.4.156/orca-linux.AppImage" and
  .size == 201856738 and
  .sha256 == "f6c394fd20ccdacd61a583f45cbd2e328d4240b06f1bc42142be0f3f58d1ba9b" and
  .extracted_tree_sha256 == "09d43fbbe1a08da9f2b3c7716af7e2a56ee8ff30688d9c6ec66e72954f30822a" and
  .source_commit == "e6b89208a69436bf856d572c4a17c98a4c1940d2"
' "$orca_release" >/dev/null

orca_installer=stacks/orca-home/scripts/install.sh
if [ ! -x "$orca_installer" ]; then
  printf '%s\n' 'Orca installer must be executable' >&2
  exit 1
fi
bash -n "$orca_installer"
orca_runner=stacks/orca-home/scripts/run.sh
if [ ! -x "$orca_runner" ]; then
  printf '%s\n' 'Orca private-output runner must be executable' >&2
  exit 1
fi
bash -n "$orca_runner"
for fragment in \
  'umask 0077' \
  '/usr/bin/install -d -m 0700' \
  '/usr/bin/chmod 0600' \
  'exec "$@" >"$readiness"'; do
  if ! grep -Fq -- "$fragment" "$orca_runner"; then
    printf 'Orca private-output runner contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
orca_updater=stacks/orca-home/scripts/update-latest.sh
if [ ! -x "$orca_updater" ]; then
  printf '%s\n' 'Orca latest-channel updater must be executable' >&2
  exit 1
fi
bash -n "$orca_updater"
for fragment in \
  '--proto-redir' \
  'sha256sum --check --status' \
  '--appimage-extract' \
  'squashfs-root/AppRun' \
  'install_lock=$install_root/.install.lock' \
  'flock --exclusive 9' \
  'mv -T -- "$staging_dir" "$release_dir"' \
  'verify_extracted_tree "$release_dir"' \
  'verify_extracted_tree "$staging_dir"' \
  'verify_dynamic_release "$install_root/$current_target"' \
  'Preserving verified auto-updated Orca release' \
  'orca-update-latest.timer' \
  'systemctl --user show-environment' \
  'state_root=${service_xdg_state_home:-$HOME/.local/state}' \
  'default_project_path=$HOME/Documents/Programming/home-server-infra' \
  'bootstrap_default_project' \
  'ORCA_PAIRING_CODE=$(jq -er' \
  'repo add --path "$default_project_path" --json' \
  'worktree list --repo "id:$repo_id" --json' \
  ': >"$readiness"' \
  '.type == "orca_server_ready"' \
  '.pairing.scope == "runtime"' \
  'systemctl --user restart orca-serve.service'; do
  if ! grep -Fq -- "$fragment" "$orca_installer"; then
    printf 'Orca installer contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
for fragment in \
  'https://api.github.com/repos/stablyai/orca/releases/latest' \
  'latest-linux.yml' \
  'sha256:' \
  'base64 --decode' \
  'sha512sum -- "$candidate"' \
  'schema_version: "orca-home.dynamic-release.v1"' \
  'install_lock=$install_root/.install.lock' \
  'flock --exclusive 9' \
  'umask 0077' \
  'systemctl --user stop orca-serve.service' \
  'stop_managed_daemon' \
  'daemon-v*.pid' \
  'expected one active daemon record' \
  'daemon command mismatch for PID' \
  '.config/orca' \
  '.config/Orca' \
  'orca-home.rollback.incomplete' \
  'profilesTarSha256' \
  'prune_incomplete_rollbacks' \
  'prune_retained_state' \
  'validate_rollback_bundle' \
  'local layout=${2:-final}' \
  'verify_dynamic_release "$candidate"' \
  'verify_dynamic_release "$staging_dir" staging' \
  'preserving unverified dynamic release' \
  'rollback_activation' \
  'update-blocked.json' \
  'probe_websocket http://127.0.0.1:6768/' \
  'probe_websocket https://orca.dongwontuna.net/' \
  'verify_default_project' \
  'current runtime selector is not a symlink' \
  'preserving the operator stop'; do
  if ! grep -Fq -- "$fragment" "$orca_updater"; then
    printf 'Orca updater contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done

orca_service=stacks/orca-home/systemd/orca-serve.service
for fragment in \
  'ExecStart=/usr/bin/bash %h/.local/libexec/orca-home-run %h/.local/orca/current/squashfs-root/AppRun --no-sandbox serve --port 6768 --pairing-address wss://orca.dongwontuna.net --json' \
  'Environment=LIBGL_ALWAYS_SOFTWARE=1' \
  'Environment=APPDIR=%h/.local/orca/current/squashfs-root' \
  'UnsetEnvironment=DISPLAY' \
  'StateDirectory=orca-home' \
  'StateDirectoryMode=0700' \
  'Restart=always' \
  'UMask=0077' \
  'NoNewPrivileges=true' \
  'PrivateTmp=true' \
  'ProtectSystem=full' \
  'ReadOnlyPaths=%h/.local/orca/releases' \
  'StandardOutput=null' \
  'StandardError=journal'; do
  if ! grep -Fq -- "$fragment" "$orca_service"; then
    printf 'Orca service contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
if [ "$(grep -Fo -- '--no-sandbox' "$orca_service" | wc -l)" -ne 1 ]; then
  printf '%s\n' 'Orca service must declare exactly one --no-sandbox fallback' >&2
  exit 1
fi
if grep -Fq -- '--mobile-pairing' "$orca_service"; then
  printf '%s\n' 'Orca home must issue a remote runtime pairing, not a mobile-only pairing' >&2
  exit 1
fi
if grep -Fq 'APPIMAGE_EXTRACT_AND_RUN' "$orca_service" \
  || grep -Fq -- '--appimage-extract-and-run' "$orca_installer"; then
  printf '%s\n' 'Orca service must use the one-time extracted AppRun path' >&2
  exit 1
fi
if grep -Eq '^StandardOutput=(journal|journal-or-kmsg|inherit|file:)' "$orca_service"; then
  printf '%s\n' 'Orca pairing output must never enter the journal' >&2
  exit 1
fi

orca_update_service=stacks/orca-home/systemd/orca-update-latest.service
for fragment in \
  'Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin' \
  'ExecStart=%h/.local/libexec/orca-home-update-latest --apply' \
  'TimeoutStartSec=20min' \
  'UMask=0077' \
  'Nice=10' \
  'IOSchedulingClass=idle' \
  'NoNewPrivileges=true' \
  'PrivateTmp=true' \
  'ProtectSystem=full' \
  'StandardOutput=journal' \
  'StandardError=journal'; do
  if ! grep -Fq -- "$fragment" "$orca_update_service"; then
    printf 'Orca update service contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done

orca_update_timer=stacks/orca-home/systemd/orca-update-latest.timer
for fragment in \
  'OnCalendar=hourly' \
  'Persistent=true' \
  'RandomizedDelaySec=10m' \
  'AccuracySec=1m' \
  'Unit=orca-update-latest.service' \
  'WantedBy=timers.target'; do
  if ! grep -Fq -- "$fragment" "$orca_update_timer"; then
    printf 'Orca update timer contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

scripts/scan-secrets.sh
scripts/test-credential-scan.sh
for retired in \
  scripts/quarantine-hermes-credentials.sh \
  scripts/test-quarantine-hermes-credentials.sh \
  scripts/verify-nvidia-build-lb-stack.py; do
  if git ls-files --error-unmatch "$retired" >/dev/null 2>&1; then
    printf 'Retired NVIDIA/Hermes helper is still tracked: %s\n' "$retired" >&2
    exit 1
  fi
done
CODEX_LB_POSTGRES_PASSWORD=placeholder \
  docker compose -f stacks/codex-lb/compose.yaml config --format json \
  >"$tmpdir/codex-lb-compose.json"
docker compose -f stacks/maintenance/compose.yaml config >/dev/null
tunnel_config=stacks/tunnel-apps/cloudflared/tunnel-apps.yml
awk '
  function emit() {
    if (service != "") print hostname "|" path "|" service
    hostname = ""
    path = ""
    service = ""
  }
  /^[[:space:]]+- hostname:/ {
    emit()
    hostname = $0
    sub(/^[[:space:]]+- hostname:[[:space:]]*/, "", hostname)
    next
  }
  /^[[:space:]]+path:/ {
    path = $0
    sub(/^[[:space:]]+path:[[:space:]]*/, "", path)
    next
  }
  /^[[:space:]]+- service:/ {
    emit()
    service = $0
    sub(/^[[:space:]]+- service:[[:space:]]*/, "", service)
    emit()
    next
  }
  /^[[:space:]]+service:/ {
    service = $0
    sub(/^[[:space:]]+service:[[:space:]]*/, "", service)
    emit()
  }
  END { emit() }
' "$tunnel_config" >"$tmpdir/tunnel-rules.actual"
cat >"$tmpdir/tunnel-rules.expected" <<'EOF'
relay-ai.dongwontuna.net||http://localhost:2455
orca.dongwontuna.net||http://localhost:6768
||http_status:404
EOF
if ! diff -u "$tmpdir/tunnel-rules.expected" "$tmpdir/tunnel-rules.actual"; then
  printf 'Shared tunnel rule order drifted\n' >&2
  exit 1
fi

python3 - "$tmpdir/codex-lb-compose.json" dotfiles/codex/config.toml <<'PY'
import json
import sys
import tomllib

EXPECTED_IMAGE = (
    "ghcr.io/soju06/codex-lb:1.23.0@"
    "sha256:3b369ac16e03d31b940be35dd8e61583f9cbd74bfca5e92e4bd03bfec62e5b13"
)
EXPECTED_PROVIDER = {
    "name": "openai",
    "base_url": "http://127.0.0.1:2455/backend-api/codex",
    "wire_api": "responses",
    "env_key": "CODEX_LB_HOME_API_KEY",
    "supports_websockets": True,
    "requires_openai_auth": True,
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


with open(sys.argv[1], encoding="utf-8") as handle:
    compose = json.load(handle)
service = compose["services"]["codex-lb"]
require(service.get("image") == EXPECTED_IMAGE, "codex-lb image pin changed")
require(service.get("pull_policy") == "missing", "codex-lb pull policy must remain missing")
require(
    service.get("labels", {}).get("com.centurylinklabs.watchtower.enable") == "false",
    "codex-lb must remain excluded from Watchtower",
)
environment = service.get("environment", {})
require(
    str(environment.get("CODEX_LB_PROXY_ACCOUNT_RESPONSE_CREATE_LIMIT")) == "0",
    "single-user codex-lb must disable the local per-account response-create cap",
)
require(
    str(environment.get("CODEX_LB_PROXY_ACCOUNT_STREAM_LIMIT")) == "0",
    "single-user codex-lb must disable the local per-account stream cap",
)
ports = service.get("ports", [])
require(len(ports) == 1, "codex-lb must expose exactly one port mapping")
port = ports[0]
require(
    port.get("host_ip") == "127.0.0.1"
    and port.get("target") == 2455
    and str(port.get("published")) == "2455"
    and port.get("protocol") == "tcp",
    "codex-lb must publish port 2455 on loopback only",
)
postgres = compose["services"]["postgres"]
require(
    postgres.get("labels", {}).get("com.centurylinklabs.watchtower.enable") == "false",
    "codex-lb Postgres must remain excluded from Watchtower",
)

with open(sys.argv[2], "rb") as handle:
    codex = tomllib.load(handle)
require(codex.get("model_provider") == "codex-lb", "Codex must select the codex-lb provider")
provider = codex.get("model_providers", {}).get("codex-lb")
require(provider == EXPECTED_PROVIDER, "Codex localhost WebSocket provider contract changed")
features = codex.get("features", {})
require(
    not any(key.startswith("responses_websockets") for key in features),
    "retired responses_websockets feature flag must stay absent",
)
PY

if [ -e stacks/codex-lb/cloudflared/codex-lb.yml ]; then
  printf 'Retired path still present: stacks/codex-lb/cloudflared/codex-lb.yml\n' >&2
  exit 1
fi
for compose in stacks/codex-lb/compose.yaml stacks/tunnel-apps/compose.yaml; do
  if grep -Eq 'container_name:[[:space:]]+watchtower-|^[[:space:]]+watchtower:' "$compose"; then
    printf 'Retired per-stack Watchtower service still present: %s\n' "$compose" >&2
    exit 1
  fi
done
if grep -R 'com.centurylinklabs.watchtower.scope' -n stacks >/dev/null 2>&1; then
  printf 'Retired Watchtower scope label still present under stacks/\n' >&2
  exit 1
fi
if ! grep -q 'bind=127.0.0.1' stacks/agent-stack/compose.yml; then
  printf 'SSH forwarder must bind 2222 on loopback only\n' >&2
  exit 1
fi
cloudflared_metrics_actual="$tmpdir/cloudflared-metrics.actual"
cloudflared_metrics_expected="$tmpdir/cloudflared-metrics.expected"
grep -Eho '127[.]0[.]0[.]1:202[0-9]+' \
  stacks/agent-stack/compose.yml \
  stacks/codexpro-home/systemd/cloudflared-codexpro-home.service \
  stacks/tunnel-apps/compose.yaml \
  | sort >"$cloudflared_metrics_actual"
cat >"$cloudflared_metrics_expected" <<'EOF'
127.0.0.1:20241
127.0.0.1:20242
127.0.0.1:20245
EOF
if ! diff -u "$cloudflared_metrics_expected" "$cloudflared_metrics_actual"; then
  printf '%s\n' 'Cloudflared services must reserve unique loopback metrics ports' >&2
  exit 1
fi
codexpro_tunnel_config=stacks/codexpro-home/cloudflared/codexpro-home.yml
awk '
  function emit() {
    if (service != "") print hostname "|" path "|" service
    hostname = ""
    path = ""
    service = ""
  }
  /^[[:space:]]+- hostname:/ {
    emit()
    hostname = $0
    sub(/^[[:space:]]+- hostname:[[:space:]]*/, "", hostname)
    next
  }
  /^[[:space:]]+path:/ {
    path = $0
    sub(/^[[:space:]]+path:[[:space:]]*/, "", path)
    next
  }
  /^[[:space:]]+- service:/ {
    emit()
    service = $0
    sub(/^[[:space:]]+- service:[[:space:]]*/, "", service)
    emit()
    next
  }
  /^[[:space:]]+service:/ {
    service = $0
    sub(/^[[:space:]]+service:[[:space:]]*/, "", service)
    emit()
  }
  END { emit() }
' "$codexpro_tunnel_config" >"$tmpdir/codexpro-tunnel-rules.actual"
cat >"$tmpdir/codexpro-tunnel-rules.expected" <<'EOF'
codexpro.dongwontuna.net|^/mcp$|http://127.0.0.1:8788
||http_status:404
EOF
if ! diff -u \
  "$tmpdir/codexpro-tunnel-rules.expected" \
  "$tmpdir/codexpro-tunnel-rules.actual"; then
  printf '%s\n' 'CodexPro public ingress must expose exactly /mcp and then return 404' >&2
  exit 1
fi
if ! grep -Fqx 'tunnel: efdf4f6b-c5ee-4673-b682-eda9a0ef71ca' "$codexpro_tunnel_config"; then
  printf '%s\n' 'CodexPro tunnel ID drifted from the deployed named tunnel' >&2
  exit 1
fi
if grep -Eq 'codexpro_token=|Authorization:[[:space:]]*Bearer' "$codexpro_tunnel_config"; then
  printf '%s\n' 'CodexPro tunnel config must not contain connector credentials' >&2
  exit 1
fi

codexpro_service=stacks/codexpro-home/systemd/codexpro-home.service
cloudflared_codexpro_service=stacks/codexpro-home/systemd/cloudflared-codexpro-home.service
for fragment in \
  'ExecStart=%h/.local/bin/codexpro start --root %h --tunnel none' \
  'ExecStartPost=%h/.local/bin/codexpro-home-url --wait 30000 --write' \
  'StandardOutput=null' \
  'CODEXPRO_BLOCKED_GLOBS='; do
  if ! grep -Fq "$fragment" "$codexpro_service"; then
    printf 'CodexPro service contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
for fragment in \
  'Requires=codexpro-home.service' \
  'PartOf=codexpro-home.service' \
  'ExecStart=%h/.local/bin/cloudflared tunnel --metrics 127.0.0.1:20242 --config %h/.cloudflared/codexpro-home.yml run codexpro-home'; do
  if ! grep -Fq "$fragment" "$cloudflared_codexpro_service"; then
    printf 'CodexPro tunnel service contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
if [ ! -x stacks/codexpro-home/scripts/codexpro-home-url.mjs ]; then
  printf '%s\n' 'CodexPro URL writer must be executable' >&2
  exit 1
fi
if ! grep -Fq 'Pass exactly one of --write or --redacted.' \
  stacks/codexpro-home/scripts/codexpro-home-url.mjs; then
  printf '%s\n' 'CodexPro URL writer must fail closed instead of printing a bearer URL by default' >&2
  exit 1
fi
node --check stacks/codexpro-home/scripts/codexpro-home-url.mjs

for retired_path in stacks/paca stacks/mcp-suite; do
  if [ -e "$retired_path" ]; then
    printf 'Retired stack path still present: %s\n' "$retired_path" >&2
    exit 1
  fi
done
if grep -Eq 'paca[.]dongwontuna[.]net|localhost:3080|127[.]0[.]0[.]1:3080|8301|8302|8303|mcp-suite' \
  stacks/tunnel-apps/cloudflared/tunnel-apps.yml; then
  printf 'Retired Paca or local MCP route remains in tunnel-apps\n' >&2
  exit 1
fi
if grep -Rq 'paca_mcp_internal' stacks/codex-lb; then
  printf 'Retired Paca network remains in codex-lb configuration\n' >&2
  exit 1
fi
if grep -Eq '^(Requires|BindsTo)=' stacks/coding/systemd/coding-tools.target; then
  printf 'Domain target must use soft Wants only: stacks/coding/systemd/coding-tools.target\n' >&2
  exit 1
fi
if ! grep -q 'Wants=codex-cli-update.timer' stacks/coding/systemd/coding-tools.target; then
  printf 'coding-tools.target must group the Codex updater timer\n' >&2
  exit 1
fi
for unit in stacks/coding/systemd/codex-cli-update.timer; do
  if ! grep -q 'PartOf=coding-tools.target' "$unit"; then
    printf 'Coding tool unit must be owned by coding-tools.target: %s\n' "$unit" >&2
    exit 1
  fi
done
docker compose -f stacks/tunnel-apps/compose.yaml config >/dev/null

cp -a stacks/codex-github-runners/. "$tmpdir/"
mkdir -p "$tmpdir/state"
printf 'placeholder\n' > "$tmpdir/state/github_pat"
cat > "$tmpdir/.env.verify" <<'VERIFY_EOF'
CODEX_RELAY_API_KEY=placeholder
CODEX_LOOP_PAT=placeholder
VERIFY_EOF
docker compose -f "$tmpdir/compose.yaml" --env-file "$tmpdir/.env.verify" config >/dev/null

printf 'Layout verification passed.\n'
