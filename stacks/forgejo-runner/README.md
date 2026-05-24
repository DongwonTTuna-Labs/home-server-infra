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

The runner mounts the Forgejo-owned `forgejo-runner_codex_home` volume into job
containers at `/home/runner/.codex` and the `forgejo-runner_codex_locks` volume
for Codex auth locks and short-lived per-job auth copies. Workflows must not
print auth files or token material.

Forgejo runner only passes bind mounts declared in `container.valid_volumes`.
Keep `/codex-runner-home` and `/codex-runner-locks` in that allowlist whenever
the Codex auth mounts are used; otherwise the runner logs `is not a valid volume`
and silently starts Codex jobs without the ChatGPT-managed auth state.

Useful checks:

```bash
docker compose -f stacks/forgejo-runner/compose.yaml config
docker run --rm --entrypoint sh dongwontuna-labs-runner:latest -lc \
  'zstd --version && node --version && npm --version && bun --version && rustc --version && cargo --version && cargo clippy --version && rustfmt --version && codex --version && python3 --version && git --version'
stacks/forgejo-runner/scripts/refresh-codex-auth.sh
```
