#!/usr/bin/env bash
# @feature installation
# @spec docs/features/installation.md
# @entrypoint portable release update
set -euo pipefail

# NEXUS_BIN selects an installed executable without assuming Cargo or a checkout location.
exec "${NEXUS_BIN:-nexus}" update "$@"
