#!/bin/sh
# @feature runtime
# @feature persistence
# @spec docs/features/runtime.md
# @spec docs/features/persistence.md
set -eu

health_url="${NEXUS_HEALTH_URL:-http://127.0.0.1:7337/readyz}"
failure_file="${NEXUS_HEALTH_FAILURE_FILE:-/tmp/nexus-readiness-failures}"
ready_file="${NEXUS_HEALTH_READY_FILE:-/tmp/nexus-readiness-observed}"
failure_limit="${NEXUS_HEALTH_FAILURE_LIMIT:-3}"
startup_failure_file="${NEXUS_HEALTH_STARTUP_FAILURE_FILE:-/tmp/nexus-startup-readiness-failures}"
startup_failure_limit="${NEXUS_HEALTH_STARTUP_FAILURE_LIMIT:-12}"
signal_pid="${NEXUS_HEALTH_SIGNAL_PID:-1}"

validate_positive_integer() {
  variable_name="$1"
  value="$2"
  case "${value}" in
    ''|*[!0-9]*|0)
      echo "${variable_name} must be a positive integer" >&2
      exit 1
      ;;
  esac
}

increment_failure_count() {
  counter_file="$1"
  failure_count=0
  if [ -r "${counter_file}" ]; then
    IFS= read -r failure_count <"${counter_file}" || failure_count=0
  fi
  case "${failure_count}" in
    ''|*[!0-9]*) failure_count=0 ;;
  esac
  failure_count=$((failure_count + 1))
  printf '%s\n' "${failure_count}" >"${counter_file}"
}

validate_positive_integer NEXUS_HEALTH_FAILURE_LIMIT "${failure_limit}"
validate_positive_integer NEXUS_HEALTH_STARTUP_FAILURE_LIMIT "${startup_failure_limit}"
validate_positive_integer NEXUS_HEALTH_SIGNAL_PID "${signal_pid}"

if curl --fail --silent --show-error --connect-timeout 2 --max-time 3 \
  "${health_url}" >/dev/null; then
  rm -f "${failure_file}" "${startup_failure_file}"
  exit 0
fi

if [ ! -s "${ready_file}" ]; then
  rm -f "${failure_file}"
  increment_failure_count "${startup_failure_file}"
  if [ "${failure_count}" -ge "${startup_failure_limit}" ]; then
    rm -f "${startup_failure_file}"
    # PID 1 ignores default-action SIGTERM before nexus-server installs its
    # async signal handler. Startup recovery must therefore be uncatchable.
    kill -KILL "${signal_pid}"
  fi
  exit 1
fi

rm -f "${startup_failure_file}"
increment_failure_count "${failure_file}"

if [ "${failure_count}" -ge "${failure_limit}" ]; then
  rm -f "${failure_file}"
  kill -TERM "${signal_pid}"
fi
exit 1
