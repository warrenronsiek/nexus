# Remote and custom-host integration

## Remote companion configuration

The installed `nexus` command consumes this TOML shape. Replace the example
endpoint, project namespace, and secret-file path with deployment values:

```toml
schema_version = 1

[client]
transport = "remote"
endpoint = "https://nexus.internal.example"
project_id = "organization/repository"
secret_file = "/absolute/path/to/nexus-client.secret"
```

Keep the bearer value in `NEXUS_SECRET` or the named private file, never in TOML,
arguments, checked-in scripts, or printed diagnostics. A credential file must be
regular, owned by the effective user, not a symlink, and no more permissive than
`0600`. `NEXUS_ENDPOINT`, `NEXUS_PROJECT_ID`, and
`NEXUS_CLIENT_SECRET_FILE` select/override remote configuration. A partial set
must still resolve to a complete valid remote client. `NEXUS_SECRET` supplies
authentication but does not by itself select remote mode.

```sh
nexus --config /absolute/path/to/client.toml config check
nexus --config /absolute/path/to/client.toml doctor
nexus --config /absolute/path/to/client.toml setup
```

`nexus daemon` intentionally refuses remote configuration. The stdio companion
forwards to the configured service without creating local database authority.
Use a stable project binding for the same logical project across machines. A
remote server does not treat a client's filesystem paths as readable server
paths. Client-side Git/script/skill enrichment is performed by the companion
beside the checkout.

The service uses PostgreSQL and exposes authenticated `/api/v1/request` and
Streamable HTTP MCP `/mcp`. It is infrastructure, not a web application.
The built-in listener is plain HTTP; same-host deployment binds loopback, while
cross-host traffic needs a protected private network or trusted TLS proxy.
Provider subprocesses are disabled in the shared server by default. Do not
enable local providers merely to make deterministic ingestion work: that flag
requires an externally isolated provider command because same-user child
processes could otherwise access service secrets.

## Host and transport contracts

Prefer the installed `nexus mcp --agent HOST_NAME` stdio companion for a custom
host that needs local checkout observation. Read its `tools/list` schemas and
preserve host/session/tool identities. For lifecycle ingestion:

- `nexus_user_prompt`: stable `session_id`, actual prompt and context.
- `nexus_pre_tool_use`: `session_id`, stable `tool_use_id`, concrete `tool_name`,
  plus available `project_root`, `agent`, `turn_id`, `model`,
  `parent_tool_use_id`, and `tool_input`.
- `nexus_post_tool_use` or `nexus_post_tool_failure`: the same invocation identity
  on completion, with the host's supported result fields.
- `nexus_session_stop`: actual stopped session, not merely a temporary idle gap.

Never substitute a generic `script` label for the host's named tool. Start and
completion use one invocation ID; a retry of the workload uses a new one.
Parents and children remain attributable. Follow current schemas rather than
assuming every lifecycle tool accepts identical context fields.

Lifecycle calls have short, permissive failure behavior and return informational
context. Use bounded asynchronous capture when wiring a host; do not add
blocking allow/deny decisions or retry the user's operation after a telemetry
failure. Explicit feature recording has its own acknowledgment contract and
must surface failure to the recording integration rather than claiming success.

A direct HTTP MCP client must perform MCP initialization and required content
negotiation, supply bearer authentication and `X-Nexus-Project-ID`, and preserve
the service-issued MCP session header. Discover the deployed protocol rather
than inventing raw request envelopes. Direct clients receive no automatic
client-filesystem enrichment; use the stdio companion or implement the feature's
explicit evidence contracts where needed.

## Integration verification

Verify the resolved transport/project and host registration, then exercise an
actual prompt and named tool. Inspect sessions/claims, usage, or work history for
the relevant feature; a service health response alone cannot prove capture.
Repeat an event delivery with its original identity to check idempotency, then
simulate unavailable capture without rerunning the workload. Report observable
gaps and leave the caller's execution policy intact.
