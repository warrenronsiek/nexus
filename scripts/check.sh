#!/usr/bin/env bash
# @feature architecture-tooling
# @spec docs/features/architecture-tooling.md
# @entrypoint repository validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

python_command="${PYTHON:-python3}"
if ! "${python_command}" -c 'import tomllib' >/dev/null 2>&1; then
  if [[ -n "${PYTHON:-}" ]]; then
    echo "${PYTHON} cannot import tomllib; use Python 3.11 or newer" >&2
    exit 1
  fi
  for candidate in python3.13 python3.12 python3.11; do
    if command -v "${candidate}" >/dev/null 2>&1 && "${candidate}" -c 'import tomllib' >/dev/null 2>&1; then
      python_command="${candidate}"
      break
    fi
  done
fi
if ! "${python_command}" -c 'import tomllib' >/dev/null 2>&1; then
  echo "Nexus validation requires Python 3.11 or newer" >&2
  exit 1
fi

if [[ "$(uname -s)" == "Darwin" && -d /Library/Developer/CommandLineTools ]]; then
  export DEVELOPER_DIR=/Library/Developer/CommandLineTools
fi

"${python_command}" scripts/feature_map.py --root . --lint
"${python_command}" scripts/complexity_analysis.py --check
"${python_command}" -m unittest tests/test_feature_map.py tests/test_complexity_analysis.py
scripts/check-ui.sh
scripts/check-pi-extension.sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
