#!/usr/bin/env bash
# @feature architecture-tooling
# @feature work-history
# @spec docs/features/architecture-tooling.md
# @spec docs/features/work-history.md
# @entrypoint repository validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"
source "${repository_root}/scripts/work-capture.sh"
if [[ -z "${NEXUS_WORK_CONTEXT:-}" ]]; then
  export NEXUS_HOST_SESSION_ID="${NEXUS_HOST_SESSION_ID:-validation:${BASHPID:-$$}}"
fi

# Deadline-sensitive tests share this policy in local, hook, and CI runs.
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"

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

nexus_work_exec lint.feature-map "Feature annotations" "${python_command}" scripts/feature_map.py --root . --lint
nexus_work_exec lint.complexity "Complexity policy" "${python_command}" scripts/complexity_analysis.py --check
nexus_work_exec test.python "Python tooling tests" "${python_command}" -m unittest tests/test_feature_map.py tests/test_complexity_analysis.py tests/test_work_github.py
nexus_work_exec test.terminal "Terminal extension validation" scripts/check-pi-extension.sh
nexus_work_exec test.docker-health "Docker health checks" scripts/test-docker-healthcheck.sh
nexus_work_exec lint.format "Rust formatting" cargo fmt --all -- --check
nexus_work_exec lint.rust "Rust Clippy" cargo clippy --all-targets -- -D warnings
nexus_work_exec lint.postgres "PostgreSQL Clippy" cargo clippy --no-default-features --features postgres --bin nexus-server -- -D warnings
# Tests own their credentials; never inherit a developer's live service/provider keys.
nexus_work_exec test.rust "Rust tests" env -u NEXUS_SECRET -u NEXUS_CLAUDE_KEY -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN \
    -u NEXUS_WORK_CONTEXT -u NEXUS_WORK_SESSION_ID -u NEXUS_PARENT_ACTIVITY_ID -u NEXUS_COMMIT_OPERATION_ID -u NEXUS_HOST_SESSION_ID \
    -u CLAUDE_CODE_OAUTH_TOKEN -u OPENAI_API_KEY -u AZURE_OPENAI_API_KEY cargo test --all-targets
