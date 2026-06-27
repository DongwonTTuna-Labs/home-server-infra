#!/usr/bin/env bash
set -euo pipefail

for port in 8301 8302 8303; do
  curl -fsS "http://127.0.0.1:${port}/ping" >/dev/null
done
