#!/usr/bin/env bash
set -euo pipefail

if [ "${CODEX_CLI_AUTO_UPDATE:-1}" != "1" ]; then
  echo "Codex CLI auto-update disabled."
  exit 0
fi

target="${CODEX_CLI_VERSION:-latest}"
package="@openai/codex@${target}"

current_version="$(
  codex --version 2>/dev/null \
    | awk '{ print $2 }' \
    | head -1
)"

latest_version="$(npm view "${package}" version --silent)"
if [ -z "${latest_version}" ]; then
  echo "Unable to resolve ${package} from npm." >&2
  exit 1
fi

if [ "${current_version}" = "${latest_version}" ]; then
  echo "Codex CLI already current: ${current_version}"
  exit 0
fi

echo "Updating Codex CLI ${current_version:-missing} -> ${latest_version}"
npm install -g --no-audit --no-fund --loglevel=error "${package}"

updated_version="$(
  codex --version 2>/dev/null \
    | awk '{ print $2 }' \
    | head -1
)"

if [ "${updated_version}" != "${latest_version}" ]; then
  echo "Codex CLI update did not apply: expected ${latest_version}, got ${updated_version:-missing}." >&2
  exit 1
fi

echo "Codex CLI updated and verified: ${updated_version}"
