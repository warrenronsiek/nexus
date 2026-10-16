#!/usr/bin/env bash
# @feature architecture-tooling
# @feature runtime
# @feature persistence
# @spec docs/features/architecture-tooling.md
# @spec docs/features/runtime.md
# @spec docs/features/persistence.md
# @entrypoint production Compose smoke validation
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"

for dependency in docker curl python3; do
  if ! command -v "${dependency}" >/dev/null 2>&1; then
    echo "Docker service smoke test requires ${dependency}" >&2
    exit 1
  fi
done
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker service smoke test requires the Docker Compose plugin" >&2
  exit 1
fi

smoke_directory="$(mktemp -d)"
compose_project="${NEXUS_TEST_COMPOSE_PROJECT:-nexus-smoke-$$}"
compose=(docker compose --project-name "${compose_project}" --file "${repository_root}/docker-compose.yml")

export NEXUS_BIND_ADDRESS="127.0.0.1"
export NEXUS_PORT="${NEXUS_PORT:-$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')}"
export POSTGRES_PASSWORD="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
export NEXUS_SECRET=""

cleanup() {
  local status=$?
  trap - EXIT
  "${compose[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "${smoke_directory}"
  exit "${status}"
}
trap cleanup EXIT

redacted_logs() {
  "${compose[@]}" logs --no-color 2>&1 \
    | sed -E 's/[[:xdigit:]]{64}/[redacted-64-hex-value]/g' >&2 || true
}

fail() {
  echo "Docker service smoke test failed: $*" >&2
  redacted_logs
  exit 1
}

base_url="http://127.0.0.1:${NEXUS_PORT}"
project_id="docker-smoke"
project_root="/workspace/docker-smoke"

wait_for_ready() {
  local attempt
  for ((attempt = 1; attempt <= 90; attempt++)); do
    if curl --fail --silent --show-error --connect-timeout 2 --max-time 3 \
      "${base_url}/readyz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  fail "${base_url}/readyz did not become ready"
}

wait_for_healthy() {
  local container_id status attempt
  container_id="$("${compose[@]}" ps --quiet nexus)"
  [[ -n "${container_id}" ]] || fail "Nexus container ID is unavailable"
  for ((attempt = 1; attempt <= 90; attempt++)); do
    status="$(docker inspect --format '{{.State.Health.Status}}' "${container_id}" 2>/dev/null || true)"
    if [[ "${status}" == "healthy" ]]; then
      return 0
    fi
    sleep 2
  done
  fail "Nexus container did not become healthy"
}

nexus_restart_count() {
  local container_id
  container_id="$("${compose[@]}" ps --quiet nexus)"
  [[ -n "${container_id}" ]] || fail "Nexus container ID is unavailable"
  docker inspect --format '{{.RestartCount}}' "${container_id}"
}

wait_for_nexus_restart_after() {
  local before="$1"
  local current attempt
  for ((attempt = 1; attempt <= 90; attempt++)); do
    current="$(nexus_restart_count)"
    if ((current > before)); then
      return 0
    fi
    sleep 2
  done
  fail "Nexus did not restart after PostgreSQL became unavailable"
}

read_service_secret() {
  "${compose[@]}" exec -T nexus sh -ec \
    'test -r "$NEXUS_SECRET_FILE"; tr -d "\r\n" < "$NEXUS_SECRET_FILE"'
}

assert_service_secret_metadata() {
  local metadata
  metadata="$("${compose[@]}" exec -T nexus sh -ec \
    'stat --format="%a:%u:%F" "$NEXUS_SECRET_FILE"')" \
    || fail "generated service secret metadata could not be inspected"
  [[ "${metadata}" == "600:10001:regular file" ]] \
    || fail "generated service secret has unsafe metadata: ${metadata}"
}

assert_authenticated_api() {
  local unauthenticated_status status_response
  unauthenticated_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
    --header 'Content-Type: application/json' \
    --data '{"method":"status","params":{},"project_id":"docker-smoke"}' \
    "${base_url}/api/v1/request")"
  [[ "${unauthenticated_status}" == "401" ]] \
    || fail "unauthenticated companion API returned ${unauthenticated_status}, expected 401"

  status_response="$(curl --fail --silent --show-error \
    --header "Authorization: Bearer ${service_secret}" \
    --header 'Content-Type: application/json' \
    --data '{"method":"status","params":{},"project_id":"docker-smoke"}' \
    "${base_url}/api/v1/request")" \
    || fail "authenticated companion API request failed"
  python3 -c \
    'import json, sys; response = json.load(sys.stdin); assert response.get("ok") is True and response.get("status") == "running", response' \
    <<<"${status_response}" \
    || fail "authenticated companion API returned an unexpected response"
}

mcp_initialize() {
  local http_status
  http_status="$(curl --silent --show-error \
    --dump-header "${smoke_directory}/mcp-headers" \
    --output "${smoke_directory}/mcp-body" \
    --write-out '%{http_code}' \
    --header "Authorization: Bearer ${service_secret}" \
    --header 'Accept: application/json, text/event-stream' \
    --header 'Content-Type: application/json' \
    --header "X-Nexus-Project-ID: ${project_id}" \
    --header "X-Nexus-Project-Root: ${project_root}" \
    --header 'X-Nexus-Agent: docker-smoke' \
    --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"docker-smoke","version":"1"}}}' \
    "${base_url}/mcp")" \
    || fail "MCP initialize request failed"
  [[ "${http_status}" == "200" ]] \
    || fail "MCP initialize returned ${http_status}, expected 200"
  python3 -c \
    'import json, sys; response = json.load(sys.stdin); assert response["result"]["protocolVersion"] == "2025-06-18", response' \
    <"${smoke_directory}/mcp-body" \
    || fail "MCP initialize returned an unexpected protocol response"
  mcp_session_id="$(awk 'tolower($1) == "mcp-session-id:" {gsub("\r", "", $2); print $2}' "${smoke_directory}/mcp-headers")"
  [[ -n "${mcp_session_id}" ]] || fail "MCP initialize omitted Mcp-Session-Id"

  http_status="$(curl --silent --show-error \
    --output /dev/null \
    --write-out '%{http_code}' \
    --header "Authorization: Bearer ${service_secret}" \
    --header 'Accept: application/json, text/event-stream' \
    --header 'Content-Type: application/json' \
    --header 'MCP-Protocol-Version: 2025-06-18' \
    --header "Mcp-Session-Id: ${mcp_session_id}" \
    --header "X-Nexus-Project-ID: ${project_id}" \
    --header "X-Nexus-Project-Root: ${project_root}" \
    --header 'X-Nexus-Agent: docker-smoke' \
    --data '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    "${base_url}/mcp")" \
    || fail "MCP initialized notification failed"
  [[ "${http_status}" == "202" ]] \
    || fail "MCP initialized notification returned ${http_status}, expected 202"
}

mcp_request() {
  local payload="$1"
  local http_status
  http_status="$(curl --silent --show-error \
    --output "${smoke_directory}/mcp-body" \
    --write-out '%{http_code}' \
    --header "Authorization: Bearer ${service_secret}" \
    --header 'Accept: application/json, text/event-stream' \
    --header 'Content-Type: application/json' \
    --header 'MCP-Protocol-Version: 2025-06-18' \
    --header "Mcp-Session-Id: ${mcp_session_id}" \
    --header "X-Nexus-Project-ID: ${project_id}" \
    --header "X-Nexus-Project-Root: ${project_root}" \
    --header 'X-Nexus-Agent: docker-smoke' \
    --data "${payload}" \
    "${base_url}/mcp")" \
    || fail "MCP request failed"
  [[ "${http_status}" == "200" ]] \
    || fail "MCP request returned ${http_status}, expected 200"
}

echo "Building and starting isolated Nexus Compose project ${compose_project}"
"${compose[@]}" up --build --detach \
  || fail "Docker Compose startup failed"
wait_for_ready
wait_for_healthy

service_secret="$(read_service_secret)" \
  || fail "generated service secret could not be read from its persistent volume"
[[ "${service_secret}" =~ ^[[:xdigit:]]{64}$ ]] \
  || fail "generated service secret does not have the expected 64-character form"
assert_service_secret_metadata
assert_authenticated_api

mcp_initialize
mcp_request '{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"nexus://status"}}'
python3 -c '
import json, sys
response = json.load(sys.stdin)
content = response["result"]["contents"][0]
assert content["uri"] == "nexus://status", response
assert json.loads(content["text"])["status"] == "running", response
' <"${smoke_directory}/mcp-body" \
  || fail "MCP status resource returned an unexpected response"

memory_marker="docker-smoke-$(python3 -c 'import uuid; print(uuid.uuid4())')"
memory_add_payload="$(python3 -c '
import json, sys
print(json.dumps({"jsonrpc":"2.0", "id":3, "method":"tools/call", "params":{"name":"nexus_memory_add", "arguments":{"scope":"project", "content":sys.argv[1]}}}))
' "${memory_marker}")"
mcp_request "${memory_add_payload}"
python3 -c \
  'import json, sys; response = json.load(sys.stdin); assert response["result"]["structuredContent"]["ok"] is True, response' \
  <"${smoke_directory}/mcp-body" \
  || fail "MCP memory tool did not persist its record"

original_secret="${service_secret}"
"${compose[@]}" restart nexus >/dev/null \
  || fail "Nexus container restart failed"
wait_for_ready
wait_for_healthy
service_secret="$(read_service_secret)" \
  || fail "service secret could not be read after Nexus restart"
[[ "${service_secret}" == "${original_secret}" ]] \
  || fail "Nexus restart replaced the generated bearer secret"
assert_authenticated_api

assert_memory_persisted() {
  local search_payload
  mcp_initialize
  search_payload="$(python3 -c '
import json, sys
print(json.dumps({"jsonrpc":"2.0", "id":4, "method":"tools/call", "params":{"name":"nexus_memory_search", "arguments":{"scope":"project", "regex":sys.argv[1], "limit":10}}}))
' "${memory_marker}")"
  mcp_request "${search_payload}"
  python3 -c '
import json, sys
response = json.load(sys.stdin)
marker = sys.argv[1]
memories = response["result"]["structuredContent"]["memories"]
assert any(memory.get("content") == marker for memory in memories), response
' "${memory_marker}" <"${smoke_directory}/mcp-body" \
    || fail "persisted MCP memory was not readable"
}

assert_memory_persisted

nexus_restarts_before_postgres="$(nexus_restart_count)"
"${compose[@]}" restart postgres >/dev/null \
  || fail "PostgreSQL container restart failed"
wait_for_nexus_restart_after "${nexus_restarts_before_postgres}"
wait_for_ready
wait_for_healthy
service_secret="$(read_service_secret)" \
  || fail "service secret could not be read after PostgreSQL recovery"
[[ "${service_secret}" == "${original_secret}" ]] \
  || fail "PostgreSQL recovery replaced the bearer secret"
assert_authenticated_api
assert_memory_persisted

echo "Docker service smoke test passed: auth, MCP, persistence, and PostgreSQL recovery"
