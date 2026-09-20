#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
transport="${1:-tcp}"
duration_seconds="${2:-86400}"
if [[ -n "${3:-}" ]]; then
  payload_bytes="$3"
elif [[ "${transport}" == "udp" ]]; then
  payload_bytes=1024
else
  payload_bytes=4096
fi
report_dir="${4:-${project_dir}/target/soak-reports}"

case "${transport}" in
  tcp|udp|kcp) ;;
  *) echo "transport must be tcp, udp, or kcp" >&2; exit 2 ;;
esac
case "${duration_seconds}" in
  ''|*[!0-9]*) echo "duration must be whole seconds" >&2; exit 2 ;;
esac

mkdir -p "${report_dir}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
report="${report_dir}/${transport}-${timestamp}.log"

{
  echo "started_utc=${timestamp}"
  echo "transport=${transport} duration_seconds=${duration_seconds} payload_bytes=${payload_bytes}"
  uname -a
  rustc --version
  cargo --version
  awk -F: '/model name/ {sub(/^[ \t]+/, "", $2); print "cpu_model=" $2; exit}' /proc/cpuinfo
  awk '/MemTotal/ {print "memory_total_kib=" $2}' /proc/meminfo
  cargo run --locked --release -p rnet-transport --example load_probe -- \
    "${transport}" 18446744073709551615 "${payload_bytes}" "${duration_seconds}"
} 2>&1 | tee "${report}"

echo "report=${report}"
