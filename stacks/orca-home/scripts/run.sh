#!/usr/bin/env bash
set -Eeuo pipefail

if [ "$#" -eq 0 ]; then
  printf '%s\n' 'orca-home-run: missing Orca command' >&2
  exit 2
fi

state_root=${XDG_STATE_HOME:-$HOME/.local/state}
if [[ "$state_root" != /* ]]; then
  printf '%s\n' 'orca-home-run: state root must be absolute' >&2
  exit 1
fi

state_dir=$state_root/orca-home
readiness=$state_dir/serve-ready.json
umask 0077
/usr/bin/install -d -m 0700 -- "$state_dir"
: >"$readiness"
/usr/bin/chmod 0600 -- "$readiness"

exec "$@" >"$readiness"
