#!/usr/bin/env bash
# @feature installation
# @spec docs/features/installation.md
# @entrypoint source_bootstrap

set -euo pipefail

nexus_source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
nexus_install_root="${NEXUS_INSTALL_ROOT:-${CARGO_HOME:-${HOME}/.cargo}}"

cargo install --locked --force --root "${nexus_install_root}" --path "${nexus_source_root}"
"${nexus_install_root}/bin/nexus" setup
