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
