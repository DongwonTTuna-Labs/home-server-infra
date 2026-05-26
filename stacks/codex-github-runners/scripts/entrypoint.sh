#!/usr/bin/env bash
set -euo pipefail

runner_home="${RUNNER_HOME:-/home/runner/actions-runner}"
runner_workdir="${RUNNER_WORKDIR:-/home/runner/_work}"
github_url="${GITHUB_URL:-https://github.com}"
runner_labels="${RUNNER_LABELS:-dongwontuna-labs-runner}"
runner_org="${RUNNER_ORG:-}"
runner_group="${RUNNER_GROUP:-}"

cd "${runner_home}"
mkdir -p "${runner_workdir}" "${RUNNER_TOOL_CACHE:-${runner_workdir}/_tool}" "${RUNNER_TEMP:-${runner_workdir}/_temp}"

if [ -z "${runner_org}" ] && [ -z "${REPO_FULL_NAME:-}" ]; then
  echo "RUNNER_ORG or REPO_FULL_NAME is required." >&2
  exit 1
fi

if [ -n "${runner_org}" ] && [ -n "${REPO_FULL_NAME:-}" ]; then
  echo "Set only one of RUNNER_ORG or REPO_FULL_NAME." >&2
  exit 1
fi

if [ -z "${RUNNER_NAME:-}" ]; then
  echo "RUNNER_NAME is required, for example home-server-runner-01." >&2
  exit 1
fi

if [ -z "${GITHUB_PAT:-}" ] && [ -r /run/secrets/github_pat ]; then
  GITHUB_PAT="$(tr -d '\r\n' < /run/secrets/github_pat)"
fi

cat > "${runner_home}/.env" <<EOF
RUNNER_NAME=${RUNNER_NAME}
RUNNER_TOOL_CACHE=${RUNNER_TOOL_CACHE:-${runner_workdir}/_tool}
RUNNER_TEMP=${RUNNER_TEMP:-${runner_workdir}/_temp}
EOF

if [ ! -f "${runner_home}/.runner" ]; then
  if [ -z "${GITHUB_PAT:-}" ]; then
    echo "GITHUB_PAT is required for first-time runner registration." >&2
    exit 1
  fi

  if [ -n "${runner_org}" ]; then
    runner_target="${runner_org}"
    registration_url="https://api.github.com/orgs/${runner_org}/actions/runners/registration-token"
    config_url="${github_url}/${runner_org}"
    echo "Requesting organization registration token for ${runner_org}."
  else
    runner_target="${REPO_FULL_NAME}"
    registration_url="https://api.github.com/repos/${REPO_FULL_NAME}/actions/runners/registration-token"
    config_url="${github_url}/${REPO_FULL_NAME}"
    echo "Requesting repository registration token for ${REPO_FULL_NAME}."
  fi

  registration_response="$(
    curl -fsSL \
      -X POST \
      -H "Accept: application/vnd.github+json" \
      -H "Authorization: Bearer ${GITHUB_PAT}" \
      -H "X-GitHub-Api-Version: 2022-11-28" \
      "${registration_url}"
  )"

  registration_token="$(
    printf '%s' "${registration_response}" \
      | python3 -c 'import json, sys; print(json.load(sys.stdin)["token"])'
  )"

  echo "Configuring runner ${RUNNER_NAME} for ${runner_target}."
  config_args=(
    --unattended
    --url "${config_url}"
    --token "${registration_token}"
    --name "${RUNNER_NAME}"
    --labels "${runner_labels}"
    --work "${runner_workdir}"
    --replace
  )

  if [ -n "${runner_group}" ]; then
    config_args+=(--runnergroup "${runner_group}")
  fi

  ./config.sh "${config_args[@]}"
else
  echo "Existing runner configuration found for ${RUNNER_NAME}; reusing it."
fi

unset GITHUB_PAT registration_token registration_response
exec ./run.sh
