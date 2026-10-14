#!/usr/bin/env bash
# @feature installation
# @feature persistence
# @feature runtime
# @spec docs/features/installation.md
# @spec docs/features/persistence.md
# @spec docs/features/runtime.md
# @entrypoint one-click shared service startup
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sqlite_path="${NEXUS_SQLITE_PATH:-}"
disable_discovery=false

while (($# > 0)); do
  case "$1" in
    --sqlite)
      (($# >= 2)) || {
        echo "--sqlite requires a database path" >&2
        exit 2
      }
      sqlite_path="$2"
      shift 2
      ;;
    --no-import)
      disable_discovery=true
      sqlite_path=""
      shift
      ;;
    *)
      echo "usage: $0 [--sqlite /path/to/nexus.db] [--no-import]" >&2
      exit 2
      ;;
  esac
done

candidate_paths=()
if [[ -n "${NEXUS_DATABASE_PATH:-}" ]]; then
  candidate_paths+=("${NEXUS_DATABASE_PATH}")
fi
if [[ -n "${NEXUS_STATE_DIR:-}" ]]; then
  candidate_paths+=("${NEXUS_STATE_DIR}/nexus.db")
fi
case "$(uname -s)" in
  Darwin)
    candidate_paths+=("${HOME}/Library/Application Support/nexus/nexus.db")
    ;;
  Linux)
    candidate_paths+=("${XDG_STATE_HOME:-${HOME}/.local/state}/nexus/nexus.db")
    candidate_paths+=("${XDG_DATA_HOME:-${HOME}/.local/share}/nexus/nexus.db")
    ;;
esac

if [[ -z "${sqlite_path}" && "${disable_discovery}" == false ]]; then
  for candidate in "${candidate_paths[@]}"; do
    if [[ -f "${candidate}" ]]; then
      sqlite_path="${candidate}"
      break
    fi
  done
fi

if [[ -n "${sqlite_path}" ]]; then
  [[ -f "${sqlite_path}" ]] || {
    echo "SQLite import source is not a regular file: ${sqlite_path}" >&2
    exit 1
  }
  sqlite_path="$(cd "$(dirname "${sqlite_path}")" && pwd)/$(basename "${sqlite_path}")"
  export NEXUS_SQLITE_PATH="${sqlite_path}"
  export NEXUS_HEALTH_STARTUP_FAILURE_LIMIT="${NEXUS_HEALTH_STARTUP_FAILURE_LIMIT:-180}"
  echo "Starting Nexus and importing ${NEXUS_SQLITE_PATH} when PostgreSQL is empty"
else
  export NEXUS_SQLITE_PATH=/dev/null
  echo "Starting Nexus without a SQLite import source"
fi

exec docker compose --file "${repository_root}/docker-compose.yml" up --build --detach
