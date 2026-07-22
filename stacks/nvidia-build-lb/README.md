# NVIDIA Build Load Balancer Stack

This independent stack serves the public NVIDIA operations product and
OpenAI-compatible API on `127.0.0.1:2456`. It does not share a container,
network, database, volume, port, or credential with `codex-lb`.

## Authority and invariants

- `compose.yaml` pins the reviewed application and PostgreSQL images by
  immutable GHCR manifest digest.
- `release.json` binds those image digests to one application commit and one
  static `nblb-hermes-cutover` artifact SHA-256.
- `/opt/nvidia-build-lb/secrets/{admin_token,vault_master_key,db_password}` are
  separate root-owned regular files, mode `0600`; they never enter Git, argv,
  logs, screenshots, or Compose environment values.
- PostgreSQL and vault volumes are durable. Routine deploy/rollback never uses
  `down --volumes`.
- Watchtower is disabled. Only deliberate reviewed digest updates replace the
  images.
- Cloudflare exposes the public product, sanitized public API, health, and
  bearer-protected `/v1/*`. `/admin*`, `/internal*`, `/metrics*`, and `/debug*`
  terminate at the tunnel with empty `404`.
- Admin UI/API remains loopback-only. Do not turn the admin token into a
  Cloudflare secret.
- The Rust helper and `agent-apps-delayed-update.service` share
  `/opt/nvidia-build-lb/hermes-cutover-state/cutover.lock` across their full
  mutations. Only `agent-hermes` may be stopped by the helper.

## Required host state

- the three secret files above;
- Docker volumes `nvidia-build-lb_db-data` and `nvidia-build-lb_vault-data`;
- `/opt/agent-apps/data/hermes/.env` and `config.yaml`, root:root mode `0600`,
  regular files with one link and not mountpoints;
- `/usr/local/sbin/nblb-hermes-cutover`, installed from the exact release
  artifact and verified against `release.json`.

The vault master key, PostgreSQL data, and vault data are one recovery set.
Never rotate or restore only one member.

## Render and deploy

Before deployment, compare `release.json` and the image references in
`compose.yaml`. The raw digest has 64 lowercase hex characters; Compose owns the
`sha256:` prefix. Do not add a second mutable runtime-env copy of these values.

```bash
(
set -Eeuo pipefail
set +x

release=stacks/nvidia-build-lb/release.json
compose=stacks/nvidia-build-lb/compose.yaml
jq -e '
  .schema_version == "nblb.infra-release.v1" and
  (.app_commit | test("^[0-9a-f]{40}$")) and
  (.schema_migration | type == "number" and . == 16) and
  (.app_registry_digest | test("^[0-9a-f]{64}$")) and
  (.postgres_registry_digest | test("^[0-9a-f]{64}$")) and
  (.hermes_helper_sha256 | test("^[0-9a-f]{64}$")) and
  (.hermes_helper_run_id | type == "number" and . > 0)
' "$release" >/dev/null

app_digest=$(jq -er .app_registry_digest "$release")
postgres_digest=$(jq -er .postgres_registry_digest "$release")
schema_migration=$(jq -er .schema_migration "$release")
export NBLB_APP_REGISTRY_DIGEST=$app_digest
export NBLB_POSTGRES_REGISTRY_DIGEST=$postgres_digest
app_ref="ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$app_digest"
postgres_ref="ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$postgres_digest"

rendered=$(docker compose -f "$compose" config --format json)
jq -e --arg app "$app_ref" --arg postgres "$postgres_ref" '
  .services.app.image == $app and
  .services.migrate.image == $app and
  .services.db.image == $postgres
' <<<"$rendered" >/dev/null
curl -fsS -o /dev/null http://127.0.0.1:2455/health

registry_config=$(mktemp -d)
cleanup_registry_auth() {
  if [ -d "$registry_config" ]; then
    find -P "$registry_config" -mindepth 1 -delete
    rmdir "$registry_config"
  fi
}
trap cleanup_registry_auth EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
registry_user=$(gh api user --jq .login)
gh auth token |
  docker --config "$registry_config" login ghcr.io \
    --username "$registry_user" --password-stdin >/dev/null
docker --config "$registry_config" compose -f "$compose" pull
docker image inspect "$app_ref" "$postgres_ref" >/dev/null
docker --config "$registry_config" logout ghcr.io >/dev/null
cleanup_registry_auth
trap - EXIT HUP INT TERM

docker compose -f "$compose" up -d --pull never
docker compose -f "$compose" ps
app_id=$(docker compose -f "$compose" ps -q app)
db_id=$(docker compose -f "$compose" ps -q db)
test "$(docker inspect --format '{{.Config.Image}}' "$app_id")" = "$app_ref"
test "$(docker inspect --format '{{.Config.Image}}' "$db_id")" = "$postgres_ref"
test "$(docker inspect --format '{{.State.Health.Status}}' "$db_id")" = healthy
test "$(docker compose -f "$compose" exec -T db \
  psql -U nvidia_build_lb -d nvidia_build_lb -Atqc \
  'SELECT max(version) FROM _sqlx_migrations')" = "$schema_migration"
curl -fsS -o /dev/null http://127.0.0.1:2456/health/live
curl -fsS -o /dev/null http://127.0.0.1:2455/health
)
```

The expected runtime split is:

- `/health/live`: `200` whenever the process is alive;
- `/health/ready`: `200` only with PostgreSQL and at least one fresh eligible
  profile proof; a fresh unconfigured stack returns `503` by design;
- `/admin/api/v2/overview`: loopback admin bearer only;
- exactly two enabled and eligible upstream slots before distribution,
  failover, live multimodal, persistence, or Hermes completion is accepted.

Preserve `codex-lb` before/after evidence:

```bash
curl -fsS -o /dev/null http://127.0.0.1:2455/health
curl -fsS -o /dev/null http://127.0.0.1:2456/health/live
curl -fsS -o /dev/null http://127.0.0.1:2455/health
```

## Static Rust helper install

The helper workflow artifact name is
`nblb-hermes-cutover-<app_commit>`. Download only that workflow run, verify the
artifact checksum, static executable type, embedded commit, then install it
atomically. Do not run a helper from a mutable checkout path.

```bash
(
set -Eeuo pipefail
set +x

release=stacks/nvidia-build-lb/release.json
commit=$(jq -er .app_commit "$release")
run_id=$(jq -er .hermes_helper_run_id "$release")
expected=$(jq -er .hermes_helper_sha256 "$release")
artifact="nblb-hermes-cutover-$commit"
stage=$(mktemp -d)
cleanup_stage() {
  if [ -d "$stage" ]; then
    find -P "$stage" -mindepth 1 -delete
    rmdir "$stage"
  fi
}
trap cleanup_stage EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

gh run view "$run_id" --repo DongwonTTuna-Labs/nvidia-build-lb \
  --json headSha,conclusion \
  --jq 'select(.headSha == "'"$commit"'" and .conclusion == "success")' |
  grep -q .
gh run download "$run_id" --repo DongwonTTuna-Labs/nvidia-build-lb \
  --name "$artifact" --dir "$stage"
helper=$stage/nblb-hermes-cutover
test -f "$helper"
test -f "$stage/nblb-hermes-cutover.sha256"
test "$(cut -d' ' -f1 "$stage/nblb-hermes-cutover.sha256")" = "$expected"
test "$(sha256sum "$helper" | cut -d' ' -f1)" = "$expected"
if readelf -lW "$helper" | grep -Eq '[[:space:]]INTERP[[:space:]]'; then
  printf '%s\n' 'helper unexpectedly has a dynamic interpreter' >&2
  exit 1
fi
if readelf -dW "$helper" | grep -Fq '(NEEDED)'; then
  printf '%s\n' 'helper unexpectedly has a dynamic dependency' >&2
  exit 1
fi
chmod 0755 "$helper"
test "$(NBLB_EXPECTED_COMMIT="$commit" "$helper" version)" = \
  "nblb-hermes-cutover $commit"

cache=/opt/nvidia-build-lb/releases/$commit
sudo install -d -o root -g root -m 0755 "$cache"
sudo install -o root -g root -m 0755 "$helper" \
  "$cache/.nblb-hermes-cutover.new"
sudo mv -f -- "$cache/.nblb-hermes-cutover.new" \
  "$cache/nblb-hermes-cutover"
sudo install -o root -g root -m 0644 "$release" "$cache/.release.json.new"
sudo mv -f -- "$cache/.release.json.new" "$cache/release.json"
test "$(sudo sha256sum "$cache/nblb-hermes-cutover" | cut -d' ' -f1)" = \
  "$expected"

sudo install -o root -g root -m 0755 "$cache/nblb-hermes-cutover" \
  /usr/local/sbin/.nblb-hermes-cutover.new
sudo mv -f -- /usr/local/sbin/.nblb-hermes-cutover.new \
  /usr/local/sbin/nblb-hermes-cutover
test "$(sha256sum /usr/local/sbin/nblb-hermes-cutover | cut -d' ' -f1)" = \
  "$expected"
test "$(NBLB_EXPECTED_COMMIT="$commit" \
  /usr/local/sbin/nblb-hermes-cutover version)" = \
  "nblb-hermes-cutover $commit"

cleanup_stage
trap - EXIT HUP INT TERM
)
```

Keep the delayed updater interlock installed. Stop its timer and wait for the
service to become inactive before replacing the wrapper/drop-in, then verify
the effective `ExecStart` and restart the timer. A failed install leaves the
timer stopped for investigation.

```bash
(
set -Eeuo pipefail
timer=agent-apps-delayed-update.timer
service=agent-apps-delayed-update.service
timer_stopped=0
install_complete=0
finish_interlock_install() {
  status=$?
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ "$timer_stopped" -eq 1 ] && \
    [ "$install_complete" -eq 0 ]; then
    sudo systemctl stop "$timer" >/dev/null 2>&1 || true
    printf '%s\n' \
      'interlock installation failed; delayed-update timer remains stopped' >&2
  fi
  exit "$status"
}
trap finish_interlock_install EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

test "$(systemctl show -p ActiveState --value "$timer")" = active
sudo systemctl stop "$timer"
timer_stopped=1
deadline=$((SECONDS + 300))
while :; do
  service_state=$(systemctl show -p ActiveState --value "$service")
  service_pid=$(systemctl show -p ExecMainPID --value "$service")
  if [ "$service_state" = inactive ] && [ "$service_pid" = 0 ]; then
    break
  fi
  test "$service_state" != failed
  test "$SECONDS" -lt "$deadline"
  sleep 1
done

sudo install -d -o root -g root -m 0755 /usr/local/libexec
sudo install -o root -g root -m 0755 \
  scripts/agent-apps-delayed-update-locked.sh \
  /usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update
sudo install -d -o root -g root -m 0755 \
  /etc/systemd/system/agent-apps-delayed-update.service.d
sudo install -o root -g root -m 0644 \
  stacks/nvidia-build-lb/systemd/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf \
  /etc/systemd/system/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf
sudo systemctl daemon-reload
systemctl show -p ExecStart --value "$service" |
  grep -Fq /usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update
sudo systemctl start "$timer"
test "$(systemctl show -p ActiveState --value "$timer")" = active
install_complete=1
)
```

## Hermes cutover and QA completion

First create a live `hermes-e2e` run in `/admin/qa`. When the run is `running`,
pass its UUID to the helper. The helper recovers any nonterminal journal,
snapshots the exact Hermes file pair, issues or rotates one scoped downstream
client, applies the candidate, verifies doctor/exact marker/tool exactly-once
and LB requests, rehearses rollback, reapplies and verifies, revokes older
cutover clients, writes the root-only receipt, then completes the armed QA run.

```bash
(
set -Eeuo pipefail
commit=$(jq -er .app_commit stacks/nvidia-build-lb/release.json)
qa_run_id=${QA_RUN_UUID:?set QA_RUN_UUID to the running hermes-e2e run}
apply_output=$(sudo NBLB_EXPECTED_COMMIT="$commit" \
  /usr/local/sbin/nblb-hermes-cutover apply --qa-run "$qa_run_id")
printf '%s\n' "$apply_output"
mapfile -t generations < <(printf '%s\n' "$apply_output" |
  sed -nE 's/^cutover: committed generation=([0-9a-f-]{36})$/\1/p')
test "${#generations[@]}" -eq 1
generation=${generations[0]}
grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' \
  <<<"$generation"
printf 'Record committed Hermes generation: %s\n' "$generation"
)
```

The QA run must finish `passed` with doctor, exact_marker, tool_task,
request_correlation, secret_scan, and rollback_rehearsal cases. A 30-minute
timeout, gateway restart, helper error, incomplete marker cleanup, uncorrelated
request, or persistence mismatch finishes `failed`. Never copy either Hermes
file manually or retry against the same terminal run.

Any NVIDIA key ever pasted into chat or stored in an ordinary Hermes backup is
retired at the provider. It is never moved into another long-lived quarantine
by an infra shell/Python helper. The Rust helper's root-only generation pair is
the sole rollback authority.

After the provider console independently confirms that the old direct NVIDIA
credential is revoked, retire only the committed generation printed above.
The confirmation flag records that external fact; it does not perform provider
revocation. Verify the durable receipt before accepting the backup as retired:

```bash
(
set -Eeuo pipefail
commit=$(jq -er .app_commit stacks/nvidia-build-lb/release.json)
generation=${GENERATION_UUID:?set GENERATION_UUID to the committed generation}
grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' \
  <<<"$generation"
sudo NBLB_EXPECTED_COMMIT="$commit" \
  /usr/local/sbin/nblb-hermes-cutover retire-backup \
  --generation "$generation" --confirm-provider-revoked
receipt=/opt/nvidia-build-lb/hermes-backup-retirement-receipts/$generation.json
sudo jq -e --arg generation "$generation" --arg commit "$commit" '
  .schema_version == "nblb.hermes-backup-retirement.v1" and
  .generation == $generation and
  .embedded_commit == $commit and
  .provider_revocation_confirmed == true and
  (.cutover_client_id | test("^[0-9a-f-]{36}$")) and
  (.snapshot_env_sha256 | test("^[0-9a-f]{64}$")) and
  (.snapshot_config_sha256 | test("^[0-9a-f]{64}$"))
' "$receipt" >/dev/null
test "$(sudo stat -c '%U:%G %a' "$receipt")" = 'root:root 600'
sudo test ! -e "/opt/nvidia-build-lb/hermes-cutover-backups/$generation"
)
```

## Public Cloudflare smoke

Validate the shared tunnel config and preserve relay/Paca routes before
recreating only `cloudflared-apps`.

```bash
(
set -Eeuo pipefail
tunnel=stacks/tunnel-apps/cloudflared/tunnel-apps.yml
curl -fsS -o /dev/null http://127.0.0.1:2455/health
curl -fsS -o /dev/null http://127.0.0.1:3080/healthz
curl -fsS -o /dev/null http://127.0.0.1:2456/health/live
cloudflared tunnel --config "$tunnel" ingress validate
docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
docker compose -f stacks/tunnel-apps/compose.yaml \
  up -d --force-recreate cloudflared-apps
curl -fsS -o /dev/null https://relay-ai.dongwontuna.net/health
curl -fsS -o /dev/null https://paca.dongwontuna.net/healthz
curl -fsS -o /dev/null https://nvidia-lb.dongwontuna.net/favicon.svg
)
```

Expected public results:

```text
200  / /status /models /docs /incidents /security
200  /api/public/v1/summary /health/live
401  /v1/models without a downstream bearer (route reached the gateway)
404  /admin /admin/api/v2/overview /internal /metrics /debug
```

Collect exact HTTP codes from `https://nvidia-lb.dongwontuna.net` and confirm
`relay-ai.dongwontuna.net` plus `paca.dongwontuna.net` remain healthy. Public
`404` bodies and headers must not contain request IDs, auth hints, or internal
route information.

## Rollback

The rollback pair in `release.json` predates additive migrations 12 through 16.
It must never run its older migrator against schema 16. Use this image-only
rollback only after a complete, checksum-verified PostgreSQL + vault-data +
sealed master-key recovery set exists and the operator explicitly sets
`NBLB_PAIRED_RECOVERY_SET_VERIFIED=1`. A different schema version requires an
isolated restore rehearsal of the exact paired recovery set instead.

```bash
(
set -Eeuo pipefail
set +x
test "${NBLB_PAIRED_RECOVERY_SET_VERIFIED:-}" = 1

release=stacks/nvidia-build-lb/release.json
compose=stacks/nvidia-build-lb/compose.yaml
current_commit=$(jq -er .app_commit "$release")
current_schema=$(jq -er .schema_migration "$release")
rollback_commit=$(jq -er .rollback.app_commit "$release")
rollback_schema=$(jq -er .rollback.schema_migration "$release")
rollback_app_digest=$(jq -er .rollback.app_registry_digest "$release")
rollback_postgres_digest=$(jq -er .rollback.postgres_registry_digest "$release")
test "$current_schema" = 16
test "$rollback_schema" = 11
test "$(NBLB_EXPECTED_COMMIT="$current_commit" \
  /usr/local/sbin/nblb-hermes-cutover version)" = \
  "nblb-hermes-cutover $current_commit"
test "$(docker compose -f "$compose" exec -T db \
  psql -U nvidia_build_lb -d nvidia_build_lb -Atqc \
  'SELECT max(version) FROM _sqlx_migrations')" = "$current_schema"

export NBLB_APP_REGISTRY_DIGEST=$rollback_app_digest
export NBLB_POSTGRES_REGISTRY_DIGEST=$rollback_postgres_digest
rollback_app_ref="ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$rollback_app_digest"
rollback_postgres_ref="ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:$rollback_postgres_digest"
registry_config=$(mktemp -d)
cleanup_registry_auth() {
  if [ -d "$registry_config" ]; then
    find -P "$registry_config" -mindepth 1 -delete
    rmdir "$registry_config"
  fi
}
trap cleanup_registry_auth EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
registry_user=$(gh api user --jq .login)
gh auth token |
  docker --config "$registry_config" login ghcr.io \
    --username "$registry_user" --password-stdin >/dev/null
docker --config "$registry_config" compose -f "$compose" pull db app
docker image inspect "$rollback_app_ref" "$rollback_postgres_ref" >/dev/null
docker --config "$registry_config" logout ghcr.io >/dev/null
cleanup_registry_auth
trap - EXIT HUP INT TERM

docker compose -f "$compose" stop app
docker compose -f "$compose" up -d --no-deps --pull never \
  --force-recreate db
db_id=$(docker compose -f "$compose" ps -q db)
deadline=$((SECONDS + 120))
until test "$(docker inspect --format '{{.State.Health.Status}}' "$db_id")" = healthy; do
  test "$SECONDS" -lt "$deadline"
  sleep 2
done
docker compose -f "$compose" up -d --no-deps --pull never \
  --force-recreate app
app_id=$(docker compose -f "$compose" ps -q app)
test "$(docker inspect --format '{{.Config.Image}}' "$db_id")" = \
  "$rollback_postgres_ref"
test "$(docker inspect --format '{{.Config.Image}}' "$app_id")" = \
  "$rollback_app_ref"
curl -fsS -o /dev/null http://127.0.0.1:2456/health/live
curl -fsS -o /dev/null http://127.0.0.1:2456/health/ready
printf 'Image-only rollback active at app commit %s; schema remains %s.\n' \
  "$rollback_commit" "$current_schema"
)
```

While the rollback app is active, the installed current-release helper commit
intentionally does not match it. Do not run the helper, change Hermes files, or
override the expected commit. This is an existing-traffic emergency mode only:
schema 13 made `upstream_keys.slot_no` and
`downstream_credentials.key_prefix` mandatory after the rollback app was built,
and schema 16 added QA provider identity. Do not create, rotate, enable, retire,
or probe upstream keys or downstream clients; do not change routing/settings or
incidents; and do not start QA while rolled back. Restore the current immutable
pair with the normal deploy block, verify readiness and persistence, then resume
admin, QA, and Hermes work. If an image-only rollback cannot become ready,
restore the complete paired recovery set in isolation; never restore one
volume, one Hermes file, or the master key independently. PR merge and DNS
deletion are not rollback steps.
