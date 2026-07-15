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
  stacks/paca/README.md
  stacks/paca/compose.yaml
  stacks/paca/docker-compose.override.yaml
  stacks/paca/.env.example
  stacks/paca/caddy/Caddyfile
  stacks/paca/relay-ai-enforce.sql
  stacks/paca/mcp-local-servers.sql
  stacks/paca/overrides/ai-agent/enforce_xhigh.py
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

assert_no_mcp_tunnel_exposure() {
  local tunnel_config=$1

  if grep -Eq '8301|8302|8303|mcp-suite|/mcp' "$tunnel_config"; then
    printf 'MCP endpoints must not be exposed through tunnel-apps: %s\n' "$tunnel_config" >&2
    exit 1
  fi
}

paca_watchtower_true_services() {
  awk '
    /^[[:space:]][[:space:]][[:alnum:]_-]+:$/ {
      service = $1
      sub(/:$/, "", service)
    }
    /com[.]centurylinklabs[.]watchtower[.]enable:[[:space:]]*"?true"?/ {
      if (service != "") print service
    }
  ' stacks/paca/compose.yaml | sort
}

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
    "ghcr.io/soju06/codex-lb:1.21.0@"
    "sha256:f8f24d08d7cb4b993e64a52ed87b8eb769788a60df8e921665e817523d0ab945"
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
if ! grep -q 'paca.dongwontuna.net' stacks/tunnel-apps/cloudflared/tunnel-apps.yml; then
  printf 'tunnel-apps must route paca.dongwontuna.net\n' >&2
  exit 1
fi
assert_no_mcp_tunnel_exposure stacks/tunnel-apps/cloudflared/tunnel-apps.yml
if ! grep -q 'paca_mcp_internal' stacks/mcp-suite/compose.yaml; then
  printf 'mcp-suite must join paca_mcp_internal\n' >&2
  exit 1
fi
for port in 8301 8302 8303; do
  if ! grep -q "127[.]0[.]0[.]1:${port}:${port}" stacks/mcp-suite/compose.yaml; then
    printf 'mcp-suite port %s must publish on loopback only\n' "$port" >&2
    exit 1
  fi
done
if ! grep -q '^name: paca$' stacks/paca/compose.yaml; then
  printf 'Paca compose project name must be paca\n' >&2
  exit 1
fi
if ! grep -Eq 'name:[[:space:]]+paca_mcp_internal' stacks/paca/compose.yaml; then
  printf 'Paca compose default network must be paca_mcp_internal\n' >&2
  exit 1
fi
if ! grep -q '127[.]0[.]0[.]1:3080:80' stacks/paca/compose.yaml; then
  printf 'Paca gateway must bind 3080 on loopback only\n' >&2
  exit 1
fi
if [ -e stacks/paca/overrides/ai-agent/builder.py ]; then
  printf 'Paca must not freeze the complete auto-updated ai-agent builder\n' >&2
  exit 1
fi
if grep -q 'builder.py:/app/src/agent/builder.py' stacks/paca/compose.yaml stacks/paca/docker-compose.override.yaml; then
  printf 'Paca compose must not bind-mount a complete ai-agent builder override\n' >&2
  exit 1
fi
paca_watchtower_true_services_actual="$(paca_watchtower_true_services)"
for service in api web realtime gateway minio valkey db-backup; do
  if printf '%s\n' "$paca_watchtower_true_services_actual" | grep -qx "$service"; then
    printf 'Paca service must not be Watchtower-enabled: %s\n' "$service" >&2
    exit 1
  fi
done
paca_watchtower_true_services_expected="$(printf '%s\n' ai-agent postgres | sort)"
if [ "$paca_watchtower_true_services_actual" != "$paca_watchtower_true_services_expected" ]; then
  printf 'Paca Watchtower true labels must be exactly ai-agent and postgres; got: %s\n' "${paca_watchtower_true_services_actual:-none}" >&2
  exit 1
fi
for port in 8301 8302 8303; do
  if ! grep -q "http://mcp-suite:${port}/mcp" stacks/paca/mcp-local-servers.sql; then
    printf 'Paca MCP seed must use mcp-suite URL for port %s\n' "$port" >&2
    exit 1
  fi
done
if grep -Eq '127[.]0[.]0[.]1:830[123]|localhost:830[123]' stacks/paca/mcp-local-servers.sql; then
  printf 'Paca MCP seed must not use host loopback URLs\n' >&2
  exit 1
fi
if ! grep -q "deleted_at IS NOT NULL" stacks/paca/relay-ai-enforce.sql; then
  printf 'Paca relay constraint must tolerate retained soft-deleted rows\n' >&2
  exit 1
fi
if ! grep -q "http://codex-lb:2455/v1" stacks/paca/relay-ai-enforce.sql; then
  printf 'Paca relay policy must route agents through local codex-lb\n' >&2
  exit 1
fi
for doc in stacks/paca/README.md docs/restore.md; do
  if ! grep -q 'relay-ai-enforce.sql' "$doc"; then
    printf 'Paca deployment docs must apply relay enforcement: %s\n' "$doc" >&2
    exit 1
  fi
  if [ "$(grep -Fc 'psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"' "$doc")" -lt 2 ]; then
    printf 'Paca SQL commands must expand database names inside the container: %s\n' "$doc" >&2
    exit 1
  fi
done
if ! grep -q 'external:[[:space:]]*true' stacks/paca/compose.yaml; then
  printf 'Paca shared network must be independently managed\n' >&2
  exit 1
fi
if ! grep -q 'enforce_xhigh.py' stacks/paca/compose.yaml; then
  printf 'Paca ai-agent must install the fail-closed xhigh patch\n' >&2
  exit 1
fi
if ! grep -q 'ghcr.io/paca-ai/paca-agent-server' stacks/paca/compose.yaml; then
  printf 'Paca sandboxes must default to the Paca agent-server image\n' >&2
  exit 1
fi
if ! grep -Fq 'pg_dump "$$DATABASE_URL" --clean --if-exists --no-owner' stacks/paca/compose.yaml; then
  printf 'Paca backups must support clean transactional restore\n' >&2
  exit 1
fi
if ! grep -A8 'Backup failed' stacks/paca/compose.yaml | grep -q 'exit 1'; then
  printf 'Paca backup failures must exit non-zero\n' >&2
  exit 1
fi
if ! grep -q -- '--single-transaction' docs/restore.md; then
  printf 'Paca restore must be transactional and fail closed\n' >&2
  exit 1
fi
if ! grep -A3 '^  default:' stacks/paca/compose.yaml | grep -q 'external:[[:space:]]*true'; then
  printf 'Paca default network must be independently managed\n' >&2
  exit 1
fi
for compose in stacks/codex-lb/compose.yaml stacks/mcp-suite/compose.yaml; do
  if ! grep -A3 '^  paca_mcp_internal:' "$compose" | grep -q 'external:[[:space:]]*true'; then
    printf 'Shared Paca network must be external in %s\n' "$compose" >&2
    exit 1
  fi
done
for doc in docs/restore.md stacks/codex-lb/README.md stacks/mcp-suite/README.md stacks/paca/README.md; do
  if ! grep -q 'docker network create paca_mcp_internal' "$doc"; then
    printf 'Shared Paca network bootstrap missing from %s\n' "$doc" >&2
    exit 1
  fi
done
cat > "$tmpdir/paca.env" <<'EOF'
ENVIRONMENT=production
PUBLIC_URL=https://paca.dongwontuna.net
COOKIE_SECURE=true
CORS_ORIGINS=https://paca.dongwontuna.net
SITE_ADDRESS=:80
POSTGRES_DB=paca
POSTGRES_USER=paca
POSTGRES_PASSWORD=placeholder
JWT_SECRET=placeholder
ADMIN_USERNAME=admin
ADMIN_PASSWORD=placeholder
ENCRYPTION_KEY=0000000000000000000000000000000000000000000000000000000000000000
AGENT_API_KEY=placeholder
INTERNAL_API_KEY=placeholder
STORAGE_PROVIDER=minio
STORAGE_ENDPOINT=minio:9000
STORAGE_PUBLIC_URL=https://paca.dongwontuna.net/storage
STORAGE_REGION=us-east-1
STORAGE_BUCKET=paca
STORAGE_ACCESS_KEY_ID=placeholder
STORAGE_SECRET_ACCESS_KEY=placeholder
STORAGE_USE_SSL=false
BACKUP_DIR=./backups
BACKUP_RETENTION_DAYS=7
BACKUP_CRON=0 2 * * *
TZ=UTC
EOF
docker compose --env-file "$tmpdir/paca.env" -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml config >/dev/null
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
