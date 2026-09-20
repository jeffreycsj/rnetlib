#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
transport="${1:-tcp}"
duration_seconds="${2:-86400}"
clients="${3:-1}"
payload_bytes="${4:-64}"
rate_per_client="${5:-20}"
report_dir="${6:-${project_dir}/target/game-soak-reports}"

case "${transport}" in
  tcp|udp|kcp) ;;
  *) echo "transport must be tcp, udp, or kcp" >&2; exit 2 ;;
esac
for value in "${duration_seconds}" "${clients}" "${payload_bytes}" "${rate_per_client}"; do
  case "${value}" in
    ''|*[!0-9]*) echo "duration, clients, payload, and rate must be whole numbers" >&2; exit 2 ;;
  esac
done

available_kib="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)"
if [[ -z "${available_kib}" || "${available_kib}" -lt 15728640 ]]; then
  echo "at least 15 GiB MemAvailable is required before starting the soak" >&2
  exit 2
fi

mkdir -p "${report_dir}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
report="${report_dir}/${transport}-${timestamp}.log"
{
  echo "started_utc=${timestamp}"
  echo "transport=${transport} duration_seconds=${duration_seconds} clients=${clients} payload_bytes=${payload_bytes} rate_per_client=${rate_per_client}"
  echo "revision=$(git -C "${project_dir}" rev-parse HEAD)"
  uname -a
  rustc --version
  cargo --version
  awk -F: '/model name/ {sub(/^[ \t]+/, "", $2); print "cpu_model=" $2; exit}' /proc/cpuinfo
  awk '/MemTotal/ {print "memory_total_kib=" $2}' /proc/meminfo
  cargo run --locked --release --manifest-path "${project_dir}/Cargo.toml" -p rnet-game --bin game_soak -- \
    "${transport}" "${duration_seconds}" "${clients}" "${payload_bytes}" "${rate_per_client}"
} 2>&1 | tee "${report}"

echo "report=${report}"
