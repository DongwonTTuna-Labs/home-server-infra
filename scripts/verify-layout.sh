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
  stacks/nvidia-build-lb/README.md
  stacks/nvidia-build-lb/compose.yaml
  stacks/nvidia-build-lb/release.json
  scripts/test-credential-scan.sh
  scripts/agent-apps-delayed-update-locked.sh
  stacks/nvidia-build-lb/systemd/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf
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

if ! git ls-files --error-unmatch scripts/test-credential-scan.sh >/dev/null 2>&1; then
  printf 'Credential negative sensor must be tracked: scripts/test-credential-scan.sh\n' >&2
  exit 1
fi
if ! git ls-files --error-unmatch scripts/agent-apps-delayed-update-locked.sh >/dev/null 2>&1; then
  printf 'Agent apps delayed update lock wrapper must be tracked\n' >&2
  exit 1
fi

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
docker compose -f stacks/nvidia-build-lb/compose.yaml config --format json \
  >"$tmpdir/nvidia-build-lb-compose.json"
jq -e '
  .schema_version == "nblb.infra-release.v1" and
  (.app_commit | test("^[0-9a-f]{40}$")) and
  .schema_migration == 16 and
  (.app_registry_digest | test("^[0-9a-f]{64}$")) and
  (.postgres_registry_digest | test("^[0-9a-f]{64}$")) and
  (.hermes_helper_sha256 | test("^[0-9a-f]{64}$")) and
  (.hermes_helper_run_id | type == "number" and . > 0 and floor == .) and
  (.rollback.app_commit | test("^[0-9a-f]{40}$")) and
  .rollback.app_commit != .app_commit and
  .rollback.schema_migration == 11 and
  (.rollback.app_registry_digest | test("^[0-9a-f]{64}$")) and
  (.rollback.postgres_registry_digest | test("^[0-9a-f]{64}$"))
' stacks/nvidia-build-lb/release.json >/dev/null
app_digest="$(jq -er .app_registry_digest stacks/nvidia-build-lb/release.json)"
postgres_digest="$(jq -er .postgres_registry_digest stacks/nvidia-build-lb/release.json)"
jq -e \
  --arg app "ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$app_digest" \
  --arg postgres "ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$postgres_digest" \
  '
  .name == "nvidia-build-lb" and
  (.services | keys | sort) == ["app", "db", "migrate"] and
  (.services.app | keys | sort) == [
    "cap_add", "cap_drop", "command", "depends_on", "entrypoint",
    "environment", "image", "labels", "networks", "ports", "read_only",
    "restart", "secrets", "security_opt", "stop_grace_period", "tmpfs",
    "volumes"
  ] and
  (.services.db | keys | sort) == [
    "cap_add", "cap_drop", "command", "entrypoint", "environment",
    "healthcheck", "image", "labels", "networks", "read_only", "restart",
    "secrets", "security_opt", "stop_grace_period", "tmpfs", "volumes"
  ] and
  (.services.migrate | keys | sort) == [
    "cap_add", "cap_drop", "command", "depends_on", "entrypoint",
    "environment", "image", "labels", "networks", "read_only", "restart",
    "secrets", "security_opt", "stop_grace_period", "tmpfs"
  ] and
  .services.app.image == $app and
  .services.migrate.image == $app and
  .services.db.image == $postgres and
  .services.app.command == null and
  .services.db.command == null and
  .services.migrate.command == ["/usr/local/bin/nblb-migrate"] and
  all(.services[]; .entrypoint == null) and
  .services.app.ports == [{
    "mode": "ingress", "host_ip": "127.0.0.1", "target": 2456,
    "published": "2456", "protocol": "tcp"
  }] and
  (.services.db | has("ports") | not) and
  (.services.migrate | has("ports") | not) and
  .services.app.networks == {"data": null, "egress": null} and
  .services.db.networks == {"data": null} and
  .services.migrate.networks == {"data": null} and
  .networks.data.name == "nvidia-build-lb_data" and
  .networks.data.internal == true and
  .networks.egress.name == "nvidia-build-lb_egress" and
  (.networks.egress.internal // false) == false and
  (.volumes | keys | sort) == ["db-data", "vault-data"] and
  .volumes["db-data"].name == "nvidia-build-lb_db-data" and
  .volumes["vault-data"].name == "nvidia-build-lb_vault-data" and
  .services.db.volumes == [{
    "type": "volume", "source": "db-data",
    "target": "/var/lib/postgresql/data", "volume": {}
  }] and
  .services.app.volumes == [{
    "type": "volume", "source": "vault-data",
    "target": "/var/lib/nvidia-build-lb", "volume": {}
  }] and
  (.services.migrate | has("volumes") | not) and
  (.secrets | keys | sort) == ["admin_token", "db_password", "vault_master_key"] and
  .secrets.admin_token.file == "/opt/nvidia-build-lb/secrets/admin_token" and
  .secrets.db_password.file == "/opt/nvidia-build-lb/secrets/db_password" and
  .secrets.vault_master_key.file == "/opt/nvidia-build-lb/secrets/vault_master_key" and
  .services.db.secrets == [{
    "source": "db_password", "target": "/run/canonical-secrets/db_password",
    "mode": "0400"
  }] and
  .services.migrate.secrets == .services.db.secrets and
  .services.app.secrets == [
    {"source": "admin_token", "target": "/run/canonical-secrets/admin_token", "mode": "0400"},
    {"source": "vault_master_key", "target": "/run/canonical-secrets/vault_master_key", "mode": "0400"},
    {"source": "db_password", "target": "/run/canonical-secrets/db_password", "mode": "0400"}
  ] and
  all(.services[]; .read_only == true) and
  .services.app.cap_drop == ["ALL"] and
  .services.migrate.cap_drop == ["ALL"] and
  .services.db.cap_drop == ["ALL"] and
  .services.app.cap_add == ["CHOWN", "SETGID", "SETUID", "SETPCAP"] and
  .services.migrate.cap_add == .services.app.cap_add and
  .services.db.cap_add == [
    "CHOWN", "SETGID", "SETUID", "SETPCAP", "FOWNER", "DAC_READ_SEARCH"
  ] and
  all(.services[]; .security_opt == ["no-new-privileges:true"]) and
  all(.services[];
    (has("pid") or has("ipc") or has("devices") or
     has("device_cgroup_rules") or has("privileged") or
     has("network_mode") or has("userns_mode") or has("uts") or
     has("volumes_from")) | not
  ) and
  .services.app.restart == "unless-stopped" and
  .services.db.restart == "unless-stopped" and
  .services.migrate.restart == "no" and
  all(.services[]; .stop_grace_period == "30s") and
  .services.app.labels == {
    "com.centurylinklabs.watchtower.enable": "false",
    "nvidia-build-lb.component": "gateway"
  } and
  .services.db.labels == {
    "com.centurylinklabs.watchtower.enable": "false",
    "nvidia-build-lb.backup-source": "true",
    "nvidia-build-lb.component": "database",
    "nvidia-build-lb.restore-isolated": "false"
  } and
  .services.migrate.labels == {
    "com.centurylinklabs.watchtower.enable": "false",
    "nvidia-build-lb.component": "migration"
  } and
  .services.migrate.depends_on == {
    "db": {"condition": "service_healthy", "required": true}
  } and
  .services.app.depends_on == {
    "db": {"condition": "service_healthy", "required": true},
    "migrate": {"condition": "service_completed_successfully", "required": true}
  } and
  .services.db.healthcheck == {
    "test": ["CMD", "pg_isready", "-h", "127.0.0.1", "-U", "nvidia_build_lb", "-d", "nvidia_build_lb"],
    "timeout": "3s", "interval": "10s", "retries": 12
  } and
  .services.app.environment == {
    "NBLB_ADMIN_PUBLIC_HOST": "",
    "NBLB_DATABASE_URL": "postgres://nvidia_build_lb@db/nvidia_build_lb",
    "NBLB_REQUIRE_DOWNSTREAM_TOKEN": "1",
    "NBLB_UPSTREAM_URL": "https://integrate.api.nvidia.com/v1/chat/completions",
    "NBLB_VAULT_MASTER_KEY_FILE": "/run/nvidia-build-lb/secrets/vault_master_key",
    "NVIDIA_BUILD_LB_ADMIN_ATTEMPT_MAX_ROWS": "40000",
    "NVIDIA_BUILD_LB_ADMIN_EVENT_MAX_ROWS": "100000",
    "NVIDIA_BUILD_LB_ADMIN_LEDGER_PRUNE_BATCH_SIZE": "1000",
    "NVIDIA_BUILD_LB_PUBLIC_PORT": "2456"
  } and
  .services.db.environment == {
    "PGDATA": "/var/lib/postgresql/data/pgdata",
    "POSTGRES_DB": "nvidia_build_lb",
    "POSTGRES_PASSWORD_FILE": "/run/canonical-secrets/db_password",
    "POSTGRES_USER": "nvidia_build_lb"
  } and
  .services.migrate.environment == {
    "NBLB_DATABASE_URL": "postgres://nvidia_build_lb@db/nvidia_build_lb",
    "NVIDIA_BUILD_LB_MODE": "migrate"
  } and
  .services.app.tmpfs == [
    "/run/nvidia-build-lb/secrets:rw,noexec,nosuid,nodev,size=64k,mode=0700,uid=0,gid=0",
    "/run/nvidia-build-lb/media-spool:rw,noexec,nosuid,nodev,size=256m,mode=0730,uid=0,gid=65532",
    "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777"
  ] and
  .services.migrate.tmpfs == [
    "/run/nvidia-build-lb/secrets:rw,noexec,nosuid,nodev,size=64k,mode=0700,uid=0,gid=0",
    "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777"
  ] and
  .services.db.tmpfs == [
    "/run/nvidia-build-lb/secrets:rw,noexec,nosuid,nodev,size=64k,mode=0700,uid=0,gid=0",
    "/var/run/postgresql:rw,noexec,nosuid,nodev,size=16m,mode=0775,uid=70,gid=70",
    "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777"
  ]
  ' \
  "$tmpdir/nvidia-build-lb-compose.json" >/dev/null

readme=stacks/nvidia-build-lb/README.md
for fragment in \
  'docker --config "$registry_config" compose -f "$compose" pull' \
  'docker image inspect "$app_ref" "$postgres_ref"' \
  'gh run download "$run_id"' \
  'readelf -lW "$helper"' \
  'readelf -dW "$helper"' \
  '/opt/nvidia-build-lb/releases/$commit' \
  'interlock installation failed; delayed-update timer remains stopped' \
  'retire-backup' \
  'nblb.hermes-backup-retirement.v1' \
  'NBLB_PAIRED_RECOVERY_SET_VERIFIED' \
  'up -d --no-deps --pull never' \
  'existing-traffic emergency mode only' \
  'Do not create, rotate, enable, retire,' \
  'and do not start QA while rolled back'; do
  if ! grep -Fq -- "$fragment" "$readme"; then
    printf 'NVIDIA operations contract missing from README: %s\n' "$fragment" >&2
    exit 1
  fi
done
if grep -Fq 'statically linked' "$readme"; then
  printf 'NVIDIA helper verification must use ELF metadata, not file wording\n' >&2
  exit 1
fi

awk -v output="$tmpdir" '
  /^```bash$/ {
    inside = 1
    count++
    file = sprintf("%s/nblb-readme-%02d.bash", output, count)
    next
  }
  /^```$/ && inside {
    inside = 0
    close(file)
    next
  }
  inside { print > file }
  END { print count > output "/nblb-readme-bash-count" }
' "$readme"
if [ "$(cat "$tmpdir/nblb-readme-bash-count")" -lt 8 ]; then
  printf 'NVIDIA README lost an expected executable Bash block\n' >&2
  exit 1
fi
for shell_block in "$tmpdir"/nblb-readme-*.bash; do
  bash -n "$shell_block"
done

wrapper=scripts/agent-apps-delayed-update-locked.sh
if [ ! -x "$wrapper" ]; then
  printf 'Delayed-update lock wrapper must be executable\n' >&2
  exit 1
fi
for fragment in \
  'flock -x 9' \
  '/opt/nvidia-build-lb/hermes-cutover-state' \
  'exec /opt/agent-apps/bin/check-delayed-updates --apply'; do
  if ! grep -Fq "$fragment" "$wrapper"; then
    printf 'Delayed-update lock wrapper contract missing: %s\n' "$fragment" >&2
    exit 1
  fi
done
if ! grep -Fq \
  'ExecStart=/usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update' \
  stacks/nvidia-build-lb/systemd/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf; then
  printf 'Delayed-update systemd interlock drifted\n' >&2
  exit 1
fi

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
paca.dongwontuna.net||http://localhost:3080
nvidia-lb.dongwontuna.net|^/admin(?:/.*)?$|http_status:404
nvidia-lb.dongwontuna.net|^/internal(?:/.*)?$|http_status:404
nvidia-lb.dongwontuna.net|^/metrics(?:/.*)?$|http_status:404
nvidia-lb.dongwontuna.net|^/debug(?:/.*)?$|http_status:404
nvidia-lb.dongwontuna.net|^/$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/_app/.*$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/favicon[.]svg$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/(?:status|models|docs|security)/?$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/incidents(?:/.*)?$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/api/public/v1(?:/.*)?$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/health(?:/.*)?$|http://localhost:2456
nvidia-lb.dongwontuna.net|^/v1(?:/.*)?$|http://localhost:2456
nvidia-lb.dongwontuna.net||http_status:404
||http_status:404
EOF
if ! diff -u "$tmpdir/tunnel-rules.expected" "$tmpdir/tunnel-rules.actual"; then
  printf 'Shared tunnel rule order or NVIDIA allow/deny contract drifted\n' >&2
  exit 1
fi

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
