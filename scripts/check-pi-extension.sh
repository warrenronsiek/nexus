#!/usr/bin/env bash
# @feature usage-analytics
# @feature architecture-tooling
# @spec docs/features/usage-analytics.md
# @spec docs/features/architecture-tooling.md
# @entrypoint pi capture validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

npm ci --prefix pi-extension
npm run --prefix pi-extension check
