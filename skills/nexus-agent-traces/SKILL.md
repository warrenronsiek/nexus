---
name: nexus-agent-traces
description: Set up and verify Nexus trace uploads from an application agent harness. Use when enabling trace ingest on nexus-server, wiring a custom or Koog exporter, or troubleshooting its first uploads and reads.
---

# Set up application agent traces

Identify the Nexus server, project ID, application ID, environment, and stable
agent-definition IDs. Use a staging scope and synthetic events for the initial
smoke test. The producer, CLI, and MCP reader must agree on the project ID.

Run `nexus traces schema` to verify the CLI supports traces and obtain the batch
and read schemas. From a checkout with trace support, the equivalent is
`cargo run --locked --bin nexus -- traces schema`.

## Enable ingest

1. On a trace-capable `nexus-server`, set `NEXUS_TRACE_CREDENTIALS_FILE` to the
   path of a private JSON array. Each object has `project_id`, `application_id`,
   `environment`, and `token`. Generate an independent token of at least
   32 visible ASCII characters for each scope. The file must be regular, owned
   by the server's effective user, and mode `0600`. In a container, mount the
   file read-only and set the environment variable to its in-container path.
   The supplied Nexus image runs as UID 10001. Apply the server configuration
   and restart the service. For Compose, recreate `nexus` with the updated
   environment and mount using `docker compose up -d --no-deps --force-recreate nexus`.
   Include the configured override files in the command if the stack uses them.
2. Configure the harness to send `POST /api/v1/agent-traces/events` with
   `Authorization: Bearer <producer-token>`, `X-Nexus-Project-ID`,
   `X-Nexus-Application-ID`, and `X-Nexus-Environment`. Use HTTPS outside
   loopback. The app and environment in the body must match the headers. Keep
   the token in the application's secret store. Use the configured Nexus
   reader credential for CLI/MCP and the existing SOS credential for SOS.
3. Run `nexus config check` on the diagnostic client. If it still needs
   project binding or shared-service transport, configure it before querying
   application traces. [Nexus setup guide](https://github.com/warrenronsiek/nexus/blob/main/skills/nexus-setup/SKILL.md)

## Instrument the harness

Capture model requests and results, application messages, tool start and
result, and explicit skill offers, loads, and activations at their actual
execution boundaries. Start and result share an invocation ID. Each event has
its own stable event ID. Use a fresh invocation ID for a retry that executes
the tool again. Keep the original batch and event IDs when retrying an upload.

Enqueue from the hooks and send bounded batches asynchronously. Each request
allows up to 512 events and 1 MiB of JSON. Retry `503`, timeouts, and lost
responses with the same batch and jittered backoff. A `409` means an accepted
identity was reused with different content. Observe dropped events separately
from the server's projection backlog.

Start with event metadata and redact approved prompt, argument, result, and
error content on the producer before sending it. Live payload retention
defaults to 30 days and is controlled by
`NEXUS_TRACE_PAYLOAD_RETENTION_DAYS`. Browser payload reads additionally need
`NEXUS_TRACE_BROWSER_CONTENT=1` and explicit reveal in the viewer. Backups and
exports have their own retention.

For Koog, install one `NexusTraceProcessor` in the agent's `Tracing` feature
and hook the app's skill loader or activation tool. The reference adapter uses a
bounded in-memory queue. Use a durable application outbox when events must
survive a producer restart before acknowledgement.
[Koog adapter and test instructions](https://github.com/warrenronsiek/nexus/tree/main/examples/koog-trace)

## Verify a complete path

Post a metadata-only test batch with `schema_version: 1`, a new `batch_id`,
the configured `application_id` and `environment`, and one `tool_finished`
event. For example, with application `support` and environment `staging`,
use `event_id: lookup-result-1`, `trace_id: smoke-run-1`,
`agent_id: order-assistant`, current RFC 3339 `occurred_at`,
`invocation_id: lookup-1`, `name: lookup_order`, and `outcome: succeeded`.
Choose IDs unique to this test. Use a private header file or the app's HTTP
client so the bearer stays out of shell arguments and captured output.

Confirm `202 Accepted` and replay the same bytes to verify the original
receipt. Check the projection and read the exact event in the same project:

```sh
nexus traces status --application support --environment staging
nexus traces calls --application support --environment staging --agent order-assistant --kind tool --name lookup_order
nexus traces get smoke-run-1 --application support --environment staging --event lookup-result-1
```

Expect one matching invocation ID and one event. If the call is missing,
check `pending_batches` before assuming data was dropped. With the trusted MCP
connection bound to the same project, call `nexus_traces` with these arguments:

```json
{"action":"calls","filter":{"application_id":"support","environment":"staging","agent_id":"order-assistant","kind":"tool","name":"lookup_order"}}
```

Then call `nexus_traces` with `action: get`, the same app and environment, the
returned `trace_id`, and `event_id` set to the returned `finished_event_id`.
Run one synthetic invocation through the instrumented application as well.
Find that invocation through the same reads and confirm its result or error
to verify the harness integration.

For SOS linking, submit a `trace_ref` with `application_id`, `environment`, `trace_id`, and
optionally `event_id` in the same project.

[Full trace wire contract](https://github.com/warrenronsiek/nexus/blob/main/docs/features/agent-traces.md)
