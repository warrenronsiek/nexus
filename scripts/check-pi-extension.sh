#!/usr/bin/env bash
# @feature observability-ui
# @feature architecture-tooling
# @spec docs/features/observability-ui.md
# @spec docs/features/architecture-tooling.md
# @entrypoint pi-extension validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

npm ci --prefix pi-extension
npm run --prefix pi-extension check
npm run --prefix pi-extension build

if ! git diff --exit-code -- pi-extension/dist; then
  echo "pi-extension/dist differs from the locked terminal build; rebuild and commit the generated assets" >&2
  exit 1
fi
