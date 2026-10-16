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
scripts/check-pi-extension.sh
scripts/test-docker-healthcheck.sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --features postgres --bin nexus-server -- -D warnings
# Tests own their credentials; never inherit a developer's live service/provider keys.
env -u NEXUS_SECRET -u NEXUS_CLAUDE_KEY -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN \
    -u CLAUDE_CODE_OAUTH_TOKEN -u OPENAI_API_KEY -u AZURE_OPENAI_API_KEY cargo test --all-targets
