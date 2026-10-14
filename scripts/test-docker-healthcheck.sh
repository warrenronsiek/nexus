#!/usr/bin/env bash
# @feature architecture-tooling
# @feature runtime
# @feature persistence
# @spec docs/features/architecture-tooling.md
# @spec docs/features/runtime.md
# @spec docs/features/persistence.md
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_directory="$(mktemp -d)"
signal_pid=""

cleanup() {
  if [[ -n "${signal_pid}" ]] && kill -0 "${signal_pid}" 2>/dev/null; then
    kill -TERM "${signal_pid}" 2>/dev/null || true
    wait "${signal_pid}" 2>/dev/null || true
  fi
  rm -rf "${test_directory}"
}
trap cleanup EXIT

mkdir -p "${test_directory}/bin"
printf '%s\n' '#!/bin/sh' 'exit 1' >"${test_directory}/bin/curl"
chmod 0755 "${test_directory}/bin/curl"

/bin/sh -c 'trap "exit 42" TERM; while :; do sleep 1; done' &
signal_pid=$!
export PATH="${test_directory}/bin:${PATH}"
export NEXUS_HEALTH_URL="http://healthcheck.test/readyz"
export NEXUS_HEALTH_READY_FILE="${test_directory}/ready"
export NEXUS_HEALTH_FAILURE_FILE="${test_directory}/post-ready-failures"
export NEXUS_HEALTH_STARTUP_FAILURE_FILE="${test_directory}/startup-failures"
export NEXUS_HEALTH_STARTUP_FAILURE_LIMIT=2
export NEXUS_HEALTH_SIGNAL_PID="${signal_pid}"

expect_unhealthy() {
  if /bin/sh "${repository_root}/scripts/docker-healthcheck.sh"; then
    echo "healthcheck unexpectedly reported success" >&2
    exit 1
  fi
}

expect_unhealthy
read -r first_count <"${NEXUS_HEALTH_STARTUP_FAILURE_FILE}"
[[ "${first_count}" == "1" ]] || {
  echo "startup health failure was not counted" >&2
  exit 1
}
kill -0 "${signal_pid}"

expect_unhealthy
set +e
wait "${signal_pid}" 2>/dev/null
startup_exit=$?
set -e
if [[ "${startup_exit}" -eq 0 ]]; then
  echo "startup watchdog did not terminate its target" >&2
  exit 1
fi
[[ "${startup_exit}" -eq 137 ]] || {
  echo "startup watchdog used a catchable signal (exit ${startup_exit}) instead of SIGKILL" >&2
  exit 1
}
signal_pid=""
[[ ! -e "${NEXUS_HEALTH_STARTUP_FAILURE_FILE}" ]] || {
  echo "startup failure counter was not cleared after signaling" >&2
  exit 1
}

echo "Docker healthcheck startup watchdog test passed"
