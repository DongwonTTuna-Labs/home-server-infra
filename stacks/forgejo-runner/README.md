# Forgejo runner

This stack runs the Forgejo Actions runner for `git.dongwontuna.net`.

All Forgejo workflow labels are intentionally collapsed to one local image:

```yaml
runs-on: dongwontuna-labs-runner
```

The image is built from `../codex-github-runners` and tagged inside the
Docker-in-Docker daemon as `dongwontuna-labs-runner:latest`. It includes Node.js
24.x, Bun, Rust, Cargo, Clippy, Rustfmt, the native C build toolchain,
pkg-config, OpenSSL headers, Python, Git, SSH, curl, jq, Codex CLI, and the
Codex auth guard.

The runner mounts the existing `codex-github-runners_codex_runner_01_home`
volume into job containers at `/home/runner/.codex` and the existing
`codex-github-runners_codex_runner_locks` volume at
`/var/lib/codex-runner/locks`. Workflows must not print auth files or token
material.

Useful checks:

```bash
docker compose -f stacks/forgejo-runner/compose.yaml config
docker run --rm --entrypoint sh dongwontuna-labs-runner:latest -lc \
  'node --version && npm --version && bun --version && rustc --version && cargo --version && cargo clippy --version && rustfmt --version && codex --version && python3 --version && git --version'
```
