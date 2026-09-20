#!/usr/bin/env bash
# @feature architecture-tooling
# @spec docs/features/architecture-tooling.md
# @entrypoint repository validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

if [[ "$(uname -s)" == "Darwin" && -d /Library/Developer/CommandLineTools ]]; then
  export DEVELOPER_DIR=/Library/Developer/CommandLineTools
fi

python3 scripts/feature_map.py --root . --lint
python3 scripts/complexity_analysis.py --check
python3 -m unittest tests/test_feature_map.py tests/test_complexity_analysis.py
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
