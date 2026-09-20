#!/usr/bin/env bash
# @feature architecture-tooling
# @spec docs/features/architecture-tooling.md
# @entrypoint hook installation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
git -C "${repository_root}" rev-parse --is-inside-work-tree >/dev/null
git -C "${repository_root}" config --local core.hooksPath .githooks

printf 'Configured core.hooksPath=.githooks for %s\n' "${repository_root}"
