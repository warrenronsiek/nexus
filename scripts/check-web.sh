#!/usr/bin/env bash
# @feature observability-ui
# @feature architecture-tooling
# @spec docs/features/observability-ui.md
# @spec docs/features/architecture-tooling.md
# @entrypoint browser application validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"
npm ci --prefix web --no-audit --no-fund
npm run --prefix web check
npm run --prefix web build
# Cargo, Docker, and release builds embed this exact validated bundle.
git diff --exit-code -- web/dist
