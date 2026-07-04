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

scripts/scan-secrets.sh
mkdir -p .omo/tmp
tmpdir="$(mktemp -d .omo/tmp/verify-layout.XXXXXX)"
trap 'rm -rf "$tmpdir"' EXIT

CODEX_LB_POSTGRES_PASSWORD=placeholder docker compose -f stacks/codex-lb/compose.yaml config >/dev/null
CODEX_LB_POSTGRES_PASSWORD=placeholder docker compose -f stacks/codex-lb-local/compose.yaml config >/dev/null
docker compose -f stacks/maintenance/compose.yaml config >/dev/null
if [ -e stacks/codex-lb/cloudflared/codex-lb.yml ]; then
  printf 'Retired path still present: stacks/codex-lb/cloudflared/codex-lb.yml\n' >&2
  exit 1
fi
if [ -e stacks/codex-lb-local/cloudflared/codex-lb-local.yml ]; then
  printf 'Retired path still present: stacks/codex-lb-local/cloudflared/codex-lb-local.yml\n' >&2
  exit 1
fi
for compose in stacks/codex-lb/compose.yaml stacks/codex-lb-local/compose.yaml stacks/tunnel-apps/compose.yaml; do
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
  printf 'Paca must not source-override auto-updated ai-agent code\n' >&2
  exit 1
fi
if grep -q 'overrides/ai-agent' stacks/paca/compose.yaml stacks/paca/docker-compose.override.yaml; then
  printf 'Paca compose must not bind-mount ai-agent source overrides\n' >&2
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
cat > "$tmpdir/.env.verify" <<'EOF'
CODEX_RELAY_API_KEY=placeholder
CODEX_LOOP_PAT=placeholder
EOF
docker compose -f "$tmpdir/compose.yaml" --env-file "$tmpdir/.env.verify" config >/dev/null

printf 'Layout verification passed.\n'
