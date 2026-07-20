# NVIDIA Build Load Balancer Stack

This stack runs the independent NVIDIA hosted API gateway on the home server.
It does not share a port, network, database, volume, or container with
`codex-lb`.

## Invariants

- The only host listener is `127.0.0.1:2456`; `codex-lb` keeps
  `127.0.0.1:2455`.
- Application and PostgreSQL images are pinned to the raw GHCR manifest
  digests produced by the reviewed `nvidia-build-lb` publish workflow.
- Watchtower is disabled for every service. Image and schema updates are
  deliberate operator actions.
- The data network is internal. Only the application joins a separate egress
  network for NVIDIA API calls.
- Cloudflare exposes only the OpenAI-compatible API and `/health`; the bearer-
  protected management UI/API is loopback-only and must be operated on the
  host. This prevents the admin token from becoming a public tunnel secret.
- The three credentials are distinct root-owned regular files. They never
  enter Git or Compose environment values.
- The database volume is independent and must never be removed as part of a
  routine rollback.

## Tracked and host state

Tracked:

- `compose.yaml`

Required only on the host:

- `/opt/nvidia-build-lb/secrets/admin_token`
- `/opt/nvidia-build-lb/secrets/vault_master_key`
- `/opt/nvidia-build-lb/secrets/db_password`
- Docker volume `nvidia-build-lb_db-data`
- Docker volume `nvidia-build-lb_vault-data`

The vault key and PostgreSQL data are one recovery pair. Never rotate the vault
key by itself and never treat a database-only dump as a complete backup.

## First deployment

Create all three secrets as one fail-closed bootstrap. The command refuses to
overwrite any existing secret and rolls back files it created if installation
does not complete:

```sh
set +x
sudo sh <<'ROOT_SH'
set -eu
secret_dir=/opt/nvidia-build-lb/secrets
install -d -o root -g root -m 0700 "$secret_dir"
for name in admin_token vault_master_key db_password; do
  if [ -e "$secret_dir/$name" ]; then
    printf 'Refusing to overwrite existing secret: %s\n' "$secret_dir/$name" >&2
    exit 1
  fi
done

stage=$(mktemp -d "$secret_dir/.bootstrap.XXXXXX")
installed=
cleanup() {
  if [ -d "$stage" ]; then
    for name in admin_token vault_master_key db_password; do
      rm -f -- "$stage/$name"
    done
    rmdir "$stage"
  fi
}
rollback() {
  for name in $installed; do
    rm -f -- "$secret_dir/$name"
  done
  cleanup
}
trap 'rollback; exit 1' HUP INT TERM
trap cleanup EXIT
umask 077
admin_hex=$(openssl rand -hex 32) \
  || { printf 'Admin token generation failed.\n' >&2; exit 1; }
db_hex=$(openssl rand -hex 32) \
  || { printf 'Database password generation failed.\n' >&2; exit 1; }
printf 'nblb_admin_%s' "$admin_hex" > "$stage/admin_token"
openssl rand 32 > "$stage/vault_master_key" \
  || { printf 'Vault key generation failed.\n' >&2; exit 1; }
printf '%s' "$db_hex" > "$stage/db_password"
[ "$(wc -c < "$stage/admin_token")" -eq 75 ] \
  && LC_ALL=C grep -Eq '^nblb_admin_[0-9a-f]{64}$' "$stage/admin_token" \
  || { printf 'Generated admin token is invalid.\n' >&2; exit 1; }
[ "$(wc -c < "$stage/vault_master_key")" -eq 32 ] \
  || { printf 'Generated vault key is invalid.\n' >&2; exit 1; }
[ "$(wc -c < "$stage/db_password")" -eq 64 ] \
  && LC_ALL=C grep -Eq '^[0-9a-f]{64}$' "$stage/db_password" \
  || { printf 'Generated database password is invalid.\n' >&2; exit 1; }
chown root:root "$stage"/*
chmod 0600 "$stage"/*
for name in admin_token vault_master_key db_password; do
  if ! ln "$stage/$name" "$secret_dir/$name"; then
    rollback
    exit 1
  fi
  installed="$installed $name"
done
trap - HUP INT TERM
ROOT_SH
```

Create the non-secret canonical runtime file and its stable lock. These values
must match this reviewed Compose pin; the release-matched application wrapper
uses them for every later backup, image rollback, and recreation:

```sh
set +x
sudo sh <<'ROOT_RUNTIME_SH'
set -eu
runtime_dir=/etc/nvidia-build-lb
runtime_file=$runtime_dir/runtime.env
runtime_lock=$runtime_dir/runtime.env.lock
install -d -o root -g root -m 0755 "$runtime_dir"
for path in "$runtime_file" "$runtime_lock"; do
  if [ -e "$path" ]; then
    printf 'Refusing to overwrite existing runtime state: %s\n' "$path" >&2
    exit 1
  fi
done

stage=$(mktemp -d "$runtime_dir/.bootstrap.XXXXXX")
installed=
cleanup() {
  if [ -d "$stage" ]; then
    rm -f -- "$stage/runtime.env" "$stage/runtime.env.lock"
    rmdir "$stage"
  fi
}
rollback() {
  for name in $installed; do
    rm -f -- "$runtime_dir/$name"
  done
  cleanup
}
trap 'rollback; exit 1' HUP INT TERM
trap cleanup EXIT
umask 022
printf '%s\n' \
  'NBLB_APP_REGISTRY_DIGEST=a9f7a0c6a9c53b5e7c48ee6e08e9e2092c6c60bb1aade5699673624a8644c8f8' \
  'NBLB_POSTGRES_REGISTRY_DIGEST=040caa04a2452d7231620ea815c53464f436c35e2859bb3e7002584aa9c25366' \
  'NBLB_SECRET_DIR=/opt/nvidia-build-lb/secrets' \
  'NBLB_ADMIN_EVENT_MAX_ROWS=100000' \
  'NBLB_ADMIN_ATTEMPT_MAX_ROWS=40000' \
  'NBLB_ADMIN_LEDGER_PRUNE_BATCH_SIZE=1000' > "$stage/runtime.env"
: > "$stage/runtime.env.lock"
chown root:root "$stage/runtime.env" "$stage/runtime.env.lock"
chmod 0644 "$stage/runtime.env" "$stage/runtime.env.lock"
sync -f "$stage/runtime.env"
sync -f "$stage/runtime.env.lock"
for name in runtime.env.lock runtime.env; do
  if ! ln "$stage/$name" "$runtime_dir/$name"; then
    rollback
    exit 1
  fi
  installed="$installed $name"
done
sync -f "$runtime_dir"
trap - HUP INT TERM
ROOT_RUNTIME_SH
```

The GHCR package is private. Authenticate only for the pull with a temporary
Docker configuration; this requires the local GitHub CLI credential to have
the read-only `read:packages` scope. No registry credential remains in Docker's
normal configuration afterward. If the scope is absent from `gh auth status`,
add only that scope through GitHub's browser approval flow:

```sh
gh auth refresh -h github.com -s read:packages
```

Then perform the bounded login and pull:

```sh
set -Eeuo pipefail
set +x
registry_config=$(mktemp -d)
cleanup_registry_auth() {
  find "$registry_config" -type f -delete
  find "$registry_config" -depth -type d -empty -delete
}
trap cleanup_registry_auth EXIT HUP INT TERM
registry_user=$(gh api user --jq .login)
gh auth token \
  | docker --config "$registry_config" login ghcr.io \
      --username "$registry_user" --password-stdin >/dev/null
docker --config "$registry_config" compose \
  -f stacks/nvidia-build-lb/compose.yaml pull
docker --config "$registry_config" logout ghcr.io >/dev/null
cleanup_registry_auth
trap - EXIT HUP INT TERM
```

From the repository root, preserve a before/after check for the existing relay
and bring up only this Compose project from the already pulled immutable pair:

```sh
curl --fail --silent --show-error --output /dev/null http://127.0.0.1:2455/health
docker compose -f stacks/nvidia-build-lb/compose.yaml config --quiet
docker compose -f stacks/nvidia-build-lb/compose.yaml up -d --pull never
docker compose -f stacks/nvidia-build-lb/compose.yaml ps
curl --silent --show-error --output /dev/null --write-out 'nvidia-build-lb=%{http_code}\n' \
  --header 'Host: 127.0.0.1:2456' http://127.0.0.1:2456/health
curl --fail --silent --show-error --output /dev/null http://127.0.0.1:2455/health
```

On a fresh database the gateway is reachable but intentionally returns HTTP
503 until an NVIDIA key completes `create -> probe(valid) -> enable`. HTTP 200
after the first key proves basic readiness only. Before a downstream token,
live matrix, or Hermes cutover, register both owned keys separately and require
exactly two rows with both `probe_status=valid`, enabled, and eligible.

## Ongoing operations and rollback authority

The exact application checkout at commit
`d30662084e4bec4ed3eebe7ef4fb0026ef2302f2` and its root-owned
`/etc/nvidia-build-lb/runtime.env` are the operational authority after first
deployment. Before using its scripts, verify the checkout and rendered project:

```bash
(
set -Eeuo pipefail
set +x
app_repo=/home/dongwonttuna/Documents/Programming/nvidia-build-lb
checkout_head=$(git -C "$app_repo" rev-parse HEAD)
test "$checkout_head" = d30662084e4bec4ed3eebe7ef4fb0026ef2302f2
checkout_status=$(git -C "$app_repo" status --porcelain=v1 --untracked-files=all)
test -z "$checkout_status"
export NBLB_RUNTIME_CONFIG_FILE=/etc/nvidia-build-lb/runtime.env
compose_wrapper=$app_repo/scripts/ops/production-compose.sh
"$compose_wrapper" config --quiet
project_name=$("$compose_wrapper" config --format json | jq -er '.name')
test "$project_name" = nvidia-build-lb
)
```

Use that release's `docs/RUNBOOK.md` for key administration and ledger
recovery, `docs/BACKUP_RESTORE.md` for quiesced paired backup plus isolated
restore, and `docs/ROLLBACK.md` for compatible immutable application-image
rollback. The wrapper and the tracked Compose resolve to the same project,
database volume, secret directory, port, and release pair at first deployment.
After an emergency image rollback, keep using the wrapper and open a reviewed
infra change to update this tracked digest before any direct Compose
recreation.

Hermes cutover and the enabled system `agent-apps-delayed-update.timer` share
one stable lock. Install the reviewed wrapper and systemd drop-in while the
timer is stopped, verify the effective `ExecStart` before restarting the timer,
and leave the timer stopped on any installation or verification failure:

```bash
(
set -Eeuo pipefail
timer=agent-apps-delayed-update.timer
service=agent-apps-delayed-update.service
test "$(systemctl show -p ActiveState --value "$timer")" = active
timer_stopped=0
finish_interlock_install() {
  original_status=$?
  trap - EXIT HUP INT TERM
  if [ "$original_status" -ne 0 ] && [ "$timer_stopped" -eq 1 ]; then
    printf '%s\n' \
      'interlock installation failed; delayed-update timer remains stopped' >&2
  fi
  exit "$original_status"
}
trap finish_interlock_install EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
sudo systemctl stop "$timer"
timer_stopped=1
deadline=$((SECONDS + 300))
while :; do
  service_state=$(systemctl show -p ActiveState --value "$service")
  service_pid=$(systemctl show -p ExecMainPID --value "$service")
  if [ "$service_state" = inactive ] && [ "$service_pid" = 0 ]; then
    break
  fi
  [ "$service_state" != failed ] \
    || { printf '%s\n' 'delayed updater failed before interlock installation' >&2; exit 1; }
  [ "$SECONDS" -lt "$deadline" ] \
    || { printf '%s\n' 'delayed updater did not finish before timeout' >&2; exit 1; }
  sleep 1
done
sudo install -o root -g root -m 0755 \
  -d /usr/local/libexec
sudo install -o root -g root -m 0755 \
  scripts/agent-apps-delayed-update-locked.sh \
  /usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update
sudo install -d -o root -g root -m 0755 \
  /etc/systemd/system/agent-apps-delayed-update.service.d
sudo install -o root -g root -m 0644 \
  stacks/nvidia-build-lb/systemd/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf \
  /etc/systemd/system/agent-apps-delayed-update.service.d/nblb-cutover-lock.conf
sudo systemctl daemon-reload
systemctl show -p ExecStart "$service" \
  | grep -Fq /usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update
test "$(systemctl show -p ActiveState --value "$service")" = inactive
sudo systemctl start "$timer"
test "$(systemctl show -p ActiveState --value "$timer")" = active
timer_stopped=0
trap - EXIT HUP INT TERM
)
```

The timer wrapper creates or validates the root-owned mode-`0600` cutover lock
and holds it across the entire delayed update. Do not run `agent-compose up`, a
manual image updater, or any other Hermes writer while a cutover command is in
progress. The cutover helper also rechecks that Hermes remains stopped before
each live-file rename.

The isolated restore proves recoverability but deliberately does not replace
the production database/key pair. A production state cutover is not a routine
command in this stack: retain the verified isolated generation and both current
and restored pairs, keep the live generation unchanged, and require a separate
reviewed host cutover change. Never copy a dump over the live volume, swap only
the vault key, or remove either generation. Never print request authorization
headers, request bodies, secret files, or an unfiltered container inspection
while collecting evidence.

## Legacy Hermes credential quarantine

Never execute a slice of this README with `sed -n`: line numbers are not an
operator interface and move whenever the runbook changes. Use the tracked,
named helper from a reviewed infra checkout. It prevalidates every planned
entry before moving anything, then moves only legacy
`.nblb-cutover-backup-*` directories and `sessions/request_dump_*.json` files
out of the Hermes bind mount, rejects links, hard links, or special files,
applies root-only `0700`/`0600` custody, rejects either direction of overlap
between a Hermes mount source and either cutover root, and scans the original
Hermes tree without printing any credential value or matching path:

```bash
sudo /home/dongwonttuna/Documents/Programming/home-server-infra-nvidia-build-lb/scripts/quarantine-hermes-credentials.sh
```

After downstream cutover, every successful run prints
`Hermes credential matches after quarantine: 0`. Before cutover, the live
`.env` may still contain the one expected direct credential; that state prints
`Hermes legacy credential matches after quarantine: 0` plus the safe
`direct-upstream` generation class while still rejecting any unexpected match.
A run that can move artifacts first writes and fsyncs a secret-free pending
generation receipt under the host-only state root, then prints the safe
`Legacy quarantine generation` ID. A failed move prints the same generation ID
as `requires recovery` after forcing root-only custody. Re-running resumes that
pending generation instead of inventing another ID. Every completed but
unretired generation is re-reported as `pending retirement`, so a signal or
lost terminal output cannot make its retirement authority disappear.
Quarantine is containment, not provider revocation:
replace and revoke every NVIDIA credential that ever appeared in chat or an
ordinary Hermes backup. Then retire only that printed legacy generation while
holding the same cutover lock:

```bash
sudo /home/dongwonttuna/Documents/Programming/home-server-infra-nvidia-build-lb/scripts/quarantine-hermes-credentials.sh \
  retire "$LEGACY_QUARANTINE_GENERATION" --provider-credential-revoked
```

The explicit acknowledgement is valid only after provider-side revocation.
The helper prevalidates the entire direct-child generation, rejects links,
hard links, nested mounts, special files, or wrong ownership/modes before
deletion, fsyncs the host-only parent, and writes a secret-free durable
retirement receipt. Repeating the same retirement ID returns the verified prior
PASS, including after deletion completed but the original terminal output was
lost.

## Replacement of credentials exposed outside the secret boundary

Treat every NVIDIA key that appeared in chat or an ordinary Hermes backup as
compromised. Do not reuse either value. Complete this ordered replacement
without placing a new key in chat, shell history, argv, logs, or a tracked file:

1. create two distinct replacement keys in the NVIDIA provider;
2. if both exposed keys are the current LB rows, replace them one at a time:
   select **Replace** on exposed row 1 so the UI retains its safe source ID,
   enter replacement 1 only through the one-time field, probe it `valid`, and
   enable it. In the temporary three-row state, follow the sole recommendation
   to disable and then delete that selected old row. Repeat that exact sequence
   for exposed row 2. This returns the pool to two rows after each replacement
   instead of creating an untestable four-row pool;
3. if an exposed key is not an LB row, omit deletion for that key and add only
   the missing replacement. Before continuing, identify rows by their safe
   fingerprint/ID evidence and leave exactly the two replacement rows enabled
   and eligible; never delete a row merely by position;
4. confirm exactly `2 registered / 2 eligible`, then pass the unchanged-source
   two-key live matrix;
5. complete the Hermes final `cycle` and confirm the replacement downstream
   generation, restart persistence, and alternate-key success;
6. revoke both exposed keys in the NVIDIA provider and record provider-side
   confirmation; removal of an old LB row is not provider revocation;
7. only after that confirmation, retire the exact upstream-bearing Hermes
   backup and legacy quarantine generations by their safe IDs.

If any step is unconfirmed, stop before deletion or retirement and preserve the
current generation. Provider revocation is the authority; removal from the LB
alone is not revocation.

## Hermes helper-issued cutover, rollback rehearsal, and recovery

Hermes remains on its NVIDIA provider profile and model `z-ai/glm-5.2`, but it
must not hold an NVIDIA upstream credential after cutover. The helper creates
the dedicated `models:read` plus `chat:write` downstream token itself under the
same lock as journal reconciliation and cutover. Do not issue or paste a Hermes
bearer in the owner UI or shell.
The release-matched helper at
`/home/dongwonttuna/Documents/Programming/nvidia-build-lb/scripts/ops/hermes_cutover.py`
is the only cutover writer. It stores immutable rollback generations under
`/opt/nvidia-build-lb/hermes-cutover-backups/` and its lock/journal under
`/opt/nvidia-build-lb/hermes-cutover-state/`; neither path is mounted into
Hermes. Same-directory candidate files exist only while Hermes intake is
stopped and may contain only the scoped downstream bearer.

Before live use, make the Hermes bearer file root-owned mode `0600`, run the
targeted fixture gate rather than the product's full test suite, then run the
root preflight:

```bash
(
set -Eeuo pipefail
app_repo=/home/dongwonttuna/Documents/Programming/nvidia-build-lb
test "$(git -C "$app_repo" rev-parse HEAD)" = d30662084e4bec4ed3eebe7ef4fb0026ef2302f2
test -z "$(git -C "$app_repo" status --porcelain=v1 --untracked-files=all)"
cd "$app_repo"
uv run pytest -q tests/operations/test_hermes_cutover.py
sudo chown root:root /opt/agent-apps/data/hermes/.env
sudo chmod 0600 /opt/agent-apps/data/hermes/.env
sudo /usr/bin/python3 scripts/ops/hermes_cutover.py preflight \
  | jq -e '
      .status == "PASS" and .backup_outside_mount == true and
      .delayed_update_lock_guard == true and .issuance_allowed == true
    ' >/dev/null
)
```

After preflight passes, run one full helper-issued cycle. It reconciles the
journal again under the same lock, writes a unique issuance label before the
single POST, and durably binds every created candidate ID. It snapshots both
regular files, stops only Hermes, verifies cutover, rehearses rollback, reapplies
the candidate, exercises a real tool-using agent run, one-key exclusion, and
restart persistence, and commits the new generation before revoking any prior downstream token. A direct NVIDIA
source has no prior downstream token; revoke that provider credential separately
only after this cycle succeeds. The receipt contains safe IDs only:

```bash
(
set -Eeuo pipefail
helper=/home/dongwonttuna/Documents/Programming/nvidia-build-lb/scripts/ops/hermes_cutover.py
set +e
CYCLE_RECEIPT=$(sudo /usr/bin/python3 "$helper" cycle)
CYCLE_STATUS=$?
set -e
printf '%s\n' "$CYCLE_RECEIPT" | jq -e '
  .status == "PASS" and .operation == "cycle" and
  .cutover.tool_run == true and .rollback.health == true and
  .reapply.health == true and .one_key_exclusion.alternate_succeeded == true and
  .restart.health == true and .previous_token_revoked == true and
  .final_token_active == true
' >/dev/null
BACKUP_ID=$(printf '%s\n' "$CYCLE_RECEIPT" | jq -er '.rollback_backup_id')
REAPPLY_BACKUP_ID=$(printf '%s\n' "$CYCLE_RECEIPT" | jq -er '.reapply_backup_id')
HERMES_TOKEN_ID=$(printf '%s\n' "$CYCLE_RECEIPT" | jq -er '.candidate_token_id')
printf '%s\n' "$CYCLE_RECEIPT"
exit "$CYCLE_STATUS"
)
```

If validation fails, the same invocation leaves Hermes stopped until it can
restore and verify the exact protected pair. A helper-issued candidate is then
reconciled by its durable unique label, revoked, and re-read; `revoked_at` must
be non-null. A `candidate_reconciliation_required` journal can remain only from
the retired manual-bearer flow. Its first recovery receipt returns `ACTION_REQUIRED`,
`candidate_reconciliation_required`, the safe `candidate_token_id`, and
`review_and_revoke_candidate_token`. It blocks every new issuance until the
owner UI revokes that exact Internal ID and `recover` verifies the revocation.
A nonterminal journal or `recovery_required` result forbids manual single-file
repair.

For any nonterminal or `recovery_required` receipt, preserve the receipt and run
only the journal-aware recovery command:

```bash
set +e
RECOVERY_RECEIPT=$(sudo /usr/bin/python3 \
  /home/dongwonttuna/Documents/Programming/nvidia-build-lb/scripts/ops/hermes_cutover.py \
  recover)
RECOVERY_STATUS=$?
set -e
printf '%s\n' "$RECOVERY_RECEIPT" | jq -e '
  (.operation == "recover") and (.next_action | type == "string") and
  ((.status == "PASS" and
    (.phase == "absent" or .phase == "aborted" or
     .phase == "rolled_back" or .phase == "applied" or .phase == "reapplied")) or
   (.status == "ACTION_REQUIRED" and
    .phase == "candidate_reconciliation_required" and
    .next_action == "review_and_revoke_candidate_token" and
    .manual_candidate_requires_review == true and
    .candidate_revoked_confirmed == false))
' >/dev/null
RECOVERY_PHASE=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.phase')
case "$RECOVERY_PHASE" in
  candidate_reconciliation_required)
    HERMES_TOKEN_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.candidate_token_id')
    printf '%s\n' \
      'Open Downstream tokens in the owner UI.' \
      'Open each row Evidence disclosure and compare its full Internal ID byte-for-byte.' \
      'Stop without revoking if no Internal ID exactly matches candidate_token_id.' \
      'On the exact row, choose Revoke; confirm the label and trailing 8-character ID.' \
      'Confirm Revoke token, then require the row state Revoked before running recover again.' >&2
    ;;
  applied)
    BACKUP_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.backup_id')
    HERMES_TOKEN_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.candidate_token_id')
    ;;
  reapplied)
    BACKUP_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.backup_id')
    REAPPLY_BACKUP_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.reapply_backup_id')
    HERMES_TOKEN_ID=$(printf '%s\n' "$RECOVERY_RECEIPT" | jq -er '.candidate_token_id')
    ;;
  absent|aborted|rolled_back) ;;
esac
exit "$RECOVERY_STATUS"
```

`candidate_reconciliation_required` returns only to the owner-UI revoke step;
it never authorizes issuance. In **Downstream tokens**, expand **Evidence** on
each row and compare the receipt's complete `candidate_token_id` byte-for-byte
with the row's complete **Internal ID**. A label or the short ID alone is not
authority. If no row matches exactly, stop and preserve the receipt without
revoking anything. For the exact matching row only, choose **Revoke**, verify
the confirmation names the expected label and the trailing eight characters of
that same ID, confirm **Revoke token**, and require the row state
`Revoked · Requests are rejected`. After that revoke, repeat `recover` and
require a PASS receipt with `candidate_revoked_confirmed=true` and
`manual_candidate_requires_review=false`. Only then may `aborted` or
`rolled_back` return to preflight and a new helper-issued cycle. `absent` has no candidate;
`applied` returns to verification/rollback; `reapplied` is the final-generation
gate. The receipt's `next_action` and safe IDs are authoritative when the
original command output was lost. If `recover` fails again, keep Hermes stopped, preserve the host-only
journal and backup generation unchanged, and do not retry issuance or edit one
file manually; review the safe error code and current pair hashes first.

The successful `cycle` receipt above is the normal rollback proof: it restores
and verifies the prior pair, then reapplies and verifies the final pair before
commit. Do not start the standalone `rollback` command after a completed cycle;
it remains only for journal-directed recovery of a retired, partially completed
manual cutover generation. A crash before the cycle commit decision restores
the exact prior pair; a crash after it completes the replacement and prior-token
revocation through `recover`.

The same release gate is available as `make smoke-hermes IMAGE_DIGEST=sha256:...`;
it additionally requires zero candidate files and an exact terminal
`reapplied` journal in `cleanup.json`.

`NVIDIA_API_KEY` in `/opt/agent-apps/data/hermes/.env` remains the Hermes NVIDIA
profile's credential variable, but after cutover its value is only the scoped
`nvidia-build-lb` downstream token. `model.base_url` is
`http://127.0.0.1:2456/v1`. Existing `codex-lb`, other agent-app containers,
and their container identities remain unchanged. After final reapply and
explicit provider-side revocation of any direct key retained by an old backup,
retire that exact host-only generation; never keep upstream credentials in the
Hermes data tree, Git, evidence, logs, or ordinary backups.

The retirement command refuses a nonterminal manifest, a still-active prior
downstream token, symlinks, unexpected file types, or an upstream-bearing
generation without the explicit provider-revocation acknowledgement. It writes
a root-only durable receipt before deletion, so retrying the same backup ID
returns the verified prior PASS after a lost terminal response:

```bash
sudo /usr/bin/python3 \
  /home/dongwonttuna/Documents/Programming/nvidia-build-lb/scripts/ops/hermes_cutover.py \
  retire-backup --backup-id "$BACKUP_ID" --provider-credential-revoked \
  | jq -e '.status == "PASS" and .backup_absent == true' >/dev/null
```

## Rollback hatch

For a failed first start, retain the database volume and inspect the bounded
service logs:

```sh
docker compose -f stacks/nvidia-build-lb/compose.yaml logs --no-color --since 10m app migrate db
docker compose -f stacks/nvidia-build-lb/compose.yaml down --remove-orphans
curl --fail --silent --show-error --output /dev/null http://127.0.0.1:2455/health
```

Do not add `--volumes`. Once credentials or routing state exist, rollback must
use a verified database/vault-key backup pair and a release-matched rollback
procedure; changing only an image digest is not a database rollback.
