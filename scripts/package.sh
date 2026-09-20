#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
architecture="$(uname -m)"
destination="${project_dir}/dist/linux-${architecture}"

cd "${project_dir}"
cargo build -p rnet-ffi --release
case "${destination}" in
  "${project_dir}"/dist/linux-*) ;;
  *) echo "refusing to replace unexpected package path: ${destination}" >&2; exit 1 ;;
esac
rm -rf -- "${destination}"
mkdir -p "${destination}/include" "${destination}/lib" "${destination}/cpp" \
  "${destination}/go" "${destination}/proto" "${destination}/examples/cpp"
mkdir -p "${destination}/cpp/rnet"
mkdir -p "${destination}/docs"
install -m 0644 target/release/librnet.a "${destination}/lib/librnet.a"
install -m 0755 target/release/librnet.so "${destination}/lib/librnet.so"
install -m 0644 include/rnet.h "${destination}/include/rnet.h"
install -m 0644 cpp/rnet.hpp "${destination}/cpp/rnet.hpp"
install -m 0644 cpp/rnet/*.hpp "${destination}/cpp/rnet/"
cp -R go/rnet "${destination}/go/"
install -m 0644 proto/README.md "${destination}/proto/README.md"
install -m 0644 examples/cpp/echo_smoke.cpp "${destination}/examples/cpp/echo_smoke.cpp"
install -m 0644 examples/cpp/secure_echo_smoke.cpp "${destination}/examples/cpp/secure_echo_smoke.cpp"
install -m 0644 docs/adversarial-review.md "${destination}/docs/adversarial-review.md"
install -m 0644 docs/dependency-review.md "${destination}/docs/dependency-review.md"
install -m 0644 docs/getting-started.md "${destination}/docs/getting-started.md"
install -m 0644 docs/game-networking.md "${destination}/docs/game-networking.md"
install -m 0644 docs/production-deployment.md "${destination}/docs/production-deployment.md"
install -m 0644 LICENSE README.md SECURITY.md go.mod "${destination}/"

(
  cd "${destination}"
  find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
)
