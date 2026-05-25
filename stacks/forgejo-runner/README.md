# Forgejo runner

This stack runs the Forgejo Actions runner for `git.dongwontuna.net`.

All Forgejo workflow labels are intentionally collapsed to one local image:

```yaml
runs-on: dongwontuna-labs-runner
```

The image is built from `../codex-github-runners` and tagged inside the
Docker-in-Docker daemon as `dongwontuna-labs-runner:latest`. It includes Node.js
24.x, Bun, Rust, Cargo, Clippy, Rustfmt, the native C build toolchain,
pkg-config, OpenSSL headers, Python, Git, SSH, curl, jq, and Codex CLI.

Codex review jobs authenticate through the `CODEX_LB_API_KEY` organization
secret and the codex-lb proxy. The old shared ChatGPT `auth.json` runner volume
path is intentionally removed.

Useful checks:

```bash
docker compose -f stacks/forgejo-runner/compose.yaml config
docker run --rm --entrypoint sh dongwontuna-labs-runner:latest -lc \
  'zstd --version && node --version && npm --version && bun --version && rustc --version && cargo --version && cargo clippy --version && rustfmt --version && codex --version && python3 --version && git --version'
```
