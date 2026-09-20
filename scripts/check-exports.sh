#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
expected="${project_dir}/scripts/expected-exports.txt"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

sed -nE 's/^(uint32_t |int32_t |size_t |const char \*)(rnet_[a-z0-9_]+)\(.*/\2/p' \
  "${project_dir}/include/rnet.h" | sort > "${temporary_dir}/header.txt"
nm -D --defined-only "${project_dir}/target/release/librnet.so" \
  | awk '$3 ~ /^rnet_/ {print $3}' | sort > "${temporary_dir}/library.txt"

diff -u "${expected}" "${temporary_dir}/header.txt"
diff -u "${expected}" "${temporary_dir}/library.txt"
