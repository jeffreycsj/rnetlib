#!/usr/bin/env bash
# Final-link against the shipped SDK, never the source-tree native archive.
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_name="linux-$(uname -m)"
original_package="${project_dir}/dist/${package_name}"
cd "${project_dir}"
"${PYTHON:-python3}" -B scripts/package_evidence.py verify --package "${original_package}"
mkdir -p target
test_dir="$(mktemp -d "${project_dir}/target/package-test.XXXXXX")"
# Test the locally produced archive that CI actually uploads, not just its source directory.
(
  cd "${project_dir}/dist"
  sha256sum -c "${package_name}.tar.gz.sha256"
)
mkdir "${test_dir}/sdk"
tar -xzf "${original_package}.tar.gz" -C "${test_dir}/sdk"
package_dir="${test_dir}/sdk/${package_name}"
cmp "${original_package}/SHA256SUMS" "${package_dir}/SHA256SUMS"
"${PYTHON:-python3}" -B scripts/package_evidence.py verify --package "${package_dir}"
test -x "${package_dir}/bin/game_soak"
test -x "${package_dir}/bin/game_probe"
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -I"${package_dir}/include" \
  "${package_dir}/examples/c/game_echo.c" "${package_dir}/lib/librnet.a" \
  -lpthread -ldl -lm -o "${test_dir}/c-echo"
"${CXX:-c++}" -std=c++11 -Wall -Wextra -Werror -I"${package_dir}/include" -I"${package_dir}/cpp" \
  "${package_dir}/examples/cpp/game_quickstart.cpp" "${package_dir}/lib/librnet.a" \
  -lpthread -ldl -lm -o "${test_dir}/cpp-echo"
(
  cd "${package_dir}"
  GOTOOLCHAIN=local CGO_ENABLED=1 go build -buildvcs=false -a -p 1 \
    -o "${test_dir}/go-echo" ./examples/go/game_quickstart
)
# Build managed example copies so qualification does not alter packaged sources.
cp -R "${package_dir}/csharp/RNet.Quickstart" "${test_dir}/RNet.Quickstart"
"${DOTNET:-dotnet}" build "${test_dir}/RNet.Quickstart" -c Release \
  -p:RNetDll="${package_dir}/lib/RNet.dll"
for transport in tcp udp kcp; do
  "${test_dir}/c-echo" "${transport}"
  "${test_dir}/cpp-echo" "${transport}" --one-slot
  "${test_dir}/go-echo" "${transport}" --one-slot
  LD_LIBRARY_PATH="${package_dir}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${DOTNET:-dotnet}" "${test_dir}/RNet.Quickstart/bin/Release/net8.0/RNet.Quickstart.dll" \
    "${transport}" --one-slot
done
# The child processes must execute the archived binary, not Cargo's debug binary.
RNET_GAME_PROBE_BINARY="${package_dir}/bin/game_probe" \
  cargo test -p rnet-game --test game_probe -- --test-threads=1
"${PYTHON:-python3}" -B scripts/package_evidence.py verify --package "${package_dir}"
