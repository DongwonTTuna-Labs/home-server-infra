# n8n Workflow Exports

The n8n instance itself is host state at `/opt/agent-apps` and is not tracked
here. Its workflows are, because the instance database is the only copy of them
and losing it loses the automation.

`gmail-auto-label.json` classifies incoming Gmail with `gpt-5.6-terra` through
the codex-lb relay and applies Gmail labels, optionally marking mail read or
archiving it.

## What the file is

Exactly the four fields the n8n public API accepts on update — `name`, `nodes`,
`connections`, `settings` — so it can be pushed straight back without editing.

Deliberately excluded:

- **Credentials.** Nodes reference them by id and display name only; the secret
  material never leaves the n8n instance. Restoring on a fresh instance means
  recreating the Gmail OAuth and codex-lb credentials by hand and pointing the
  nodes at the new ids.
- **`staticData`**, which holds the Gmail poll cursor and recently seen message
  ids. It is per-instance state, and replaying an old cursor would reprocess or
  skip mail.
- Timestamps, version ids, and the `activeVersion` snapshot n8n keeps alongside
  the live definition.

## Push a change back

```sh
key=$(awk -F= '/^N8N_API_KEY=/{sub(/^[^=]*=/,""); print; exit}' /opt/agent-apps/secrets/app.env)
curl -fsS -X PUT http://127.0.0.1:5678/api/v1/workflows/<workflow-id> \
  -H "X-N8N-API-KEY: $key" -H 'Content-Type: application/json' \
  --data @config/n8n/gmail-auto-label.json
```

## Re-export after editing in the UI

```sh
key=$(awk -F= '/^N8N_API_KEY=/{sub(/^[^=]*=/,""); print; exit}' /opt/agent-apps/secrets/app.env)
curl -fsS -H "X-N8N-API-KEY: $key" \
  http://127.0.0.1:5678/api/v1/workflows/<workflow-id> \
  | jq -S '{name, nodes, connections, settings}' \
  > config/n8n/gmail-auto-label.json
```

`scripts/verify-layout.sh` checks that the export still has that shape and
carries no credential material, so an export taken with the wrong flags cannot
quietly commit a secret.
