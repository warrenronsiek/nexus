#!/usr/bin/env bash
# @feature work-history
# @spec docs/features/work-history.md
# @entrypoint caller-owned environment activity example
set -euo pipefail

if [[ "$#" -lt 3 ]]; then
  printf 'Usage: %s ENVIRONMENT KIND COMMAND [ARG ...]\n' "$0" >&2
  exit 2
fi
repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${repository_root}/scripts/work-capture.sh"
export NEXUS_WORK_ENVIRONMENT="$1"
activity_kind="$2"
shift 2
nexus_work_exec "${activity_kind}" "${activity_kind} in ${NEXUS_WORK_ENVIRONMENT}" "$@"
