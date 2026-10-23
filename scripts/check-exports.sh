#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
expected="${project_dir}/scripts/expected-exports.txt"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

"${PYTHON:-python3}" - "${project_dir}/include/rnet.h" > "${temporary_dir}/header.txt" <<'PY'
from pathlib import Path
import re
import sys

source = Path(sys.argv[1]).read_text()
source = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
# Generated prototypes may wrap between their return type, name and arguments.
names = re.findall(r"(?:^|\n)\s*(?:uint32_t|int32_t|size_t|const\s+char\s*\*)\s*(rnet_[a-z0-9_]+)\s*\(", source)
print("\n".join(sorted(names)))
PY
nm -D --defined-only "${project_dir}/target/release/librnet.so" \
  | awk '$3 ~ /^rnet_/ {print $3}' | sort > "${temporary_dir}/library.txt"

diff -u "${expected}" "${temporary_dir}/header.txt"
diff -u "${expected}" "${temporary_dir}/library.txt"
