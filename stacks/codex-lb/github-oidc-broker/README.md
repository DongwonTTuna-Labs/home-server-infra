# codex-lb GitHub OIDC Broker

Small sidecar that exchanges GitHub Actions OIDC tokens for short-lived `codex-lb`
API keys. It keeps upstream `codex-lb` on `ghcr.io/soju06/codex-lb:latest` while
removing the long-lived `AI_RELAY_API_KEY` GitHub secret from Codex review jobs.

The broker only serves `/oidc/health` and `/oidc/exchange`. It does not proxy
Codex traffic.

## Security Model

- Accepts GitHub Actions OIDC JWTs from `https://token.actions.githubusercontent.com`.
- Requires audience `https://relay-ai.dongwontuna.net/github-actions`.
- Allows only private `DongwonTTuna-Labs` repositories and selected workflow files.
- Allows only workflows running from `refs/heads/main`.
- Allows only self-hosted runner jobs and the `DongwonTTuna` actor by default.
- Records each exchanged JWT hash in broker-local SQLite storage to prevent replay.
- Creates short-lived API keys through the `codex-lb` dashboard API.
- Leaves expired-key deletion to the one-shot cleanup script and user timer.

## Cleanup

Expired broker-issued keys are cleaned up separately:

```bash
python -m app.cleanup_expired_keys --dry-run
python -m app.cleanup_expired_keys
```

## Local Test

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install -r requirements-dev.txt
python -m pytest -q
```
