#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

required=(
  README.md
  docs/restore.md
  docs/secrets.md
  stacks/codex-lb/README.md
  stacks/codex-lb/compose.yaml
  stacks/mcp-suite/README.md
  stacks/mcp-suite/Dockerfile
  stacks/mcp-suite/compose.yaml
  stacks/mcp-suite/scripts/start.sh
  stacks/mcp-suite/scripts/healthcheck.sh
  stacks/mcp-suite/scripts/smoke.sh
  stacks/mcp-suite/systemd/mcp-suite-update.service
  stacks/mcp-suite/systemd/mcp-suite.target
  stacks/mcp-suite/systemd/mcp-suite-update.timer
  stacks/mcp-suite/systemd/mcp-suite-update.sh
  stacks/tunnel-apps/README.md
  stacks/tunnel-apps/compose.yaml
  stacks/tunnel-apps/cloudflared/tunnel-apps.yml
  stacks/coding/README.md
  stacks/coding/systemd/coding-tools.target
  stacks/coding/systemd/codex-cli-update.service
  stacks/coding/systemd/codex-cli-update.timer
  stacks/coding/systemd/opencode.service
  stacks/coding/systemd/opencode-update.service
  stacks/coding/systemd/opencode-update.sh
  stacks/coding/systemd/opencode-update.timer
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

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

scripts/scan-secrets.sh
CODEX_LB_POSTGRES_PASSWORD=placeholder \
  docker compose -f stacks/codex-lb/compose.yaml config --format json \
  >"$tmpdir/codex-lb-compose.json"
docker compose -f stacks/maintenance/compose.yaml config >/dev/null

python3 - "$tmpdir/codex-lb-compose.json" dotfiles/codex/config.toml <<'PY'
import json
import sys
import tomllib

EXPECTED_IMAGE = (
    "ghcr.io/soju06/codex-lb:1.21.0-beta.2@"
    "sha256:fa931eee760f3a6e8875ef7347d24993c899fedbf261066783e162f633d659ab"
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
if grep -q '/home/dongwonttuna:/home/dongwonttuna' stacks/mcp-suite/compose.yaml; then
  printf 'mcp-suite must not mount the whole home directory\n' >&2
  exit 1
fi
if grep -q '/tmp:/tmp' stacks/mcp-suite/compose.yaml stacks/mcp-suite/systemd/mcp-suite-update.sh; then
  printf 'mcp-suite must not bind-mount host /tmp\n' >&2
  exit 1
fi
if ! grep -q 'MCP_ALLOWED_WORKSPACE_ROOT' stacks/mcp-suite/compose.yaml; then
  printf 'mcp-suite must configure MCP_ALLOWED_WORKSPACE_ROOT\n' >&2
  exit 1
fi
if grep -Eq '8301|8302|8303|mcp-suite|/mcp' stacks/tunnel-apps/cloudflared/tunnel-apps.yml; then
  printf 'MCP endpoints must not be exposed through tunnel-apps\n' >&2
  exit 1
fi
for target in stacks/mcp-suite/systemd/mcp-suite.target stacks/coding/systemd/coding-tools.target; do
  if grep -Eq '^(Requires|BindsTo)=' "$target"; then
    printf 'Domain target must use soft Wants only: %s\n' "$target" >&2
    exit 1
  fi
done
if ! grep -q 'Wants=mcp-suite-update.timer' stacks/mcp-suite/systemd/mcp-suite.target; then
  printf 'mcp-suite.target must group MCP update timer\n' >&2
  exit 1
fi
if ! grep -q 'PartOf=mcp-suite.target' stacks/mcp-suite/systemd/mcp-suite-update.timer; then
  printf 'MCP update timer must be owned by mcp-suite.target\n' >&2
  exit 1
fi
if ! grep -q 'Wants=opencode.service opencode-update.timer codex-cli-update.timer' stacks/coding/systemd/coding-tools.target; then
  printf 'coding-tools.target must group OpenCode and Codex updater units\n' >&2
  exit 1
fi
for unit in stacks/coding/systemd/opencode.service stacks/coding/systemd/opencode-update.timer stacks/coding/systemd/codex-cli-update.timer; do
  if ! grep -q 'PartOf=coding-tools.target' "$unit"; then
    printf 'Coding tool unit must be owned by coding-tools.target: %s\n' "$unit" >&2
    exit 1
  fi
done
docker compose -f stacks/mcp-suite/compose.yaml config >/dev/null
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
