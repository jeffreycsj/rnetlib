#!/usr/bin/env bash
# Offline validation only: does not launch Prometheus or expose a metrics listener.
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
validator="${PROMTOOL:-promtool}"
if ! command -v "${validator}" >/dev/null 2>&1; then
  echo 'promtool is required for ops-test; install a trusted validator or set PROMTOOL' >&2
  exit 1
fi
"${validator}" --version
cd "${project_dir}/examples/operations"
"${validator}" check config prometheus.example.json
"${validator}" test rules alerts.test.json outcomes.test.json
