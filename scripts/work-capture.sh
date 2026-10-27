#!/usr/bin/env bash
# @feature work-history
# @spec docs/features/work-history.md
# @entrypoint named script activity capture

# Source this file, then wrap only named workload boundaries. Telemetry never
# retries the workload itself: its original result passes straight through.
nexus_work_binary() {
  local work_binary="${NEXUS_WORK_BINARY:-}"
  if [[ -z "${work_binary}" ]]; then
    local work_root
    work_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    if [[ -x "${work_root}/target/debug/nexus" ]]; then
      work_binary="${work_root}/target/debug/nexus"
    else
      work_binary="$(command -v nexus || true)"
    fi
  fi
  [[ -x "${work_binary}" ]] || return 1
  printf '%s' "${work_binary}"
}

nexus_work_exec() {
  local activity_kind="$1" activity_name="$2"
  shift 2
  local work_binary work_help=""
  work_binary="$(nexus_work_binary || true)"
  if [[ "${NEXUS_WORK_DISABLE_CAPTURE:-}" != 1 && -n "${work_binary}" ]]; then
    work_help="$("${work_binary}" exec --help 2>/dev/null || true)"
  fi
  case "${work_help}" in
    *--work-session*)
      if "${work_binary}" config check >/dev/null 2>&1; then
        "${work_binary}" exec --kind "${activity_kind}" --name "${activity_name}" -- "$@"
      else
        "$@"
      fi
      ;;
    *) "$@" ;;
  esac
}

nexus_work_git_hook() {
  [[ -n "${NEXUS_COMMIT_OPERATION_ID:-}" ]] || return 0
  local work_binary
  work_binary="$(nexus_work_binary || true)"
  if [[ -n "${work_binary}" ]]; then
    "${work_binary}" work git-hook "$1" >/dev/null 2>&1 || true
  fi
}
