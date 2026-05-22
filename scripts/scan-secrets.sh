#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

fail=0

check() {
  local name="$1"
  shift
  if "$@"; then
    return 0
  fi
  printf 'FAILED: %s\n' "$name" >&2
  fail=1
}

tracked_files() {
  git ls-files
}

check "no forbidden tracked paths" bash -c '
  ! git ls-files | grep -E \
    "(^|/)(\\.env|\\.forgejo-admin-token|auth\\.json|github_pat|known_hosts(\\.old)?|cert\\.pem|gitea\\.rsa|private\\.pem|id_[^/]+|.*\\.sqlite.*|.*\\.log)$|(^|/)(state|logs|postgres|backups|metadata|mirrors)(/|$)|^forgejo/|(^|/)runner/data(/|$)"
'

check "no private key material" bash -c '
  ! git grep -nE "BEGIN (RSA |OPENSSH |EC |DSA |PRIVATE )?PRIVATE KEY" -- .
'

check "no obvious token values" bash -c '
  ! git grep -nE "(ghp_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|refresh_token[\"[:space:]:=]+[A-Za-z0-9._-]{20,}|access_token[\"[:space:]:=]+[A-Za-z0-9._-]{20,}|TUNNEL_TOKEN=[A-Za-z0-9._-]{40,}|FORGEJO_TOKEN=[A-Za-z0-9._-]{40,})" -- .
'

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'Secret scan passed.\n'
