#!/usr/bin/env bash
# @feature observability-ui
# @feature architecture-tooling
# @spec docs/features/observability-ui.md
# @spec docs/features/architecture-tooling.md
# @entrypoint frontend validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

npm ci --prefix ui
npm run --prefix ui check

if ! git diff --exit-code -- ui/dist; then
  echo "ui/dist differs from the locked frontend build; rebuild and commit the generated assets" >&2
  exit 1
fi
