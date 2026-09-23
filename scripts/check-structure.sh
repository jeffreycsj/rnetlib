#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${project_dir}"

require_file() {
  if [[ ! -f "${project_dir}/$1" ]]; then
    echo "missing required module: $1" >&2
    exit 1
  fi
}

check_max_lines() {
  local limit="$1"
  shift
  local file lines
  for file in "$@"; do
    lines="$(wc -l < "${project_dir}/${file}")"
    if (( lines > limit )); then
      echo "${file} has ${lines} lines; limit is ${limit}" >&2
      exit 1
    fi
  done
}

transport_modules=(config cookie kcp lifecycle metrics runtime secure_datagram secure_kcp secure_tcp session state tcp udp)
ffi_modules=(abi endpoint events observe registry runtime session)
game_modules=(range_api range_runtime range_state realtime_runtime runtime runtime_tests)
go_modules=(config endpoint event keypair native observe runtime session types)

for module in "${transport_modules[@]}"; do
  require_file "crates/rnet-transport/src/${module}.rs"
done
for module in "${ffi_modules[@]}"; do
  require_file "crates/rnet-ffi/src/${module}.rs"
done
for module in "${game_modules[@]}"; do
  require_file "crates/rnet-game/src/${module}.rs"
done
for module in "${go_modules[@]}"; do
  require_file "go/rnet/${module}.go"
done
for header in types keypair runtime; do
  require_file "cpp/rnet/${header}.hpp"
done
require_file "go/rnet/native.h"

check_max_lines 200 crates/rnet-transport/src/lib.rs crates/rnet-ffi/src/lib.rs
check_max_lines 30 cpp/rnet.hpp
check_max_lines 600 \
  crates/rnet-transport/src/*.rs crates/rnet-ffi/src/*.rs \
  cpp/rnet/*.hpp go/rnet/*.go go/rnet/native.h
check_max_lines 800 crates/rnet-game/src/*.rs
check_max_lines 400 csharp/RNet/*.cs

for forbidden in common helpers misc utils; do
  if find "${project_dir}/crates/rnet-transport/src" \
      "${project_dir}/crates/rnet-ffi/src" -name "${forbidden}.rs" -print -quit \
      | grep -q .; then
    echo "forbidden catch-all module: ${forbidden}.rs" >&2
    exit 1
  fi
done

if [[ -e "${project_dir}/go/rnet/rnet.go" ]]; then
  echo "go/rnet/rnet.go must remain decomposed" >&2
  exit 1
fi

if grep -R -nE '^use (super|crate)::\*;' \
    "${project_dir}/crates/rnet-transport/src" \
    "${project_dir}/crates/rnet-ffi/src" >/dev/null; then
  echo "wildcard imports hide module dependencies" >&2
  exit 1
fi
