# Nexus

Nexus is a coordination service for coding agents. It observes tool activity, records advisory path claims, detects overlapping work, and gives agents context without controlling their execution. It can run as a local SQLite daemon for agents sharing one machine or as an authenticated Dockerized PostgreSQL service for agents working across compute environments.

The invariant is structural: every lifecycle-hook response permits the original tool call. Nexus emits no deny, block, approval, or input-rewrite decision.

## Agent memory

Nexus keeps deliberately authored memories in its configured store and supplies bounded, clearly delimited historical context once per agent session. Agents only decide which durable notes to add; Nexus builds disposable hierarchical summaries in the background with the configured Codex or Claude analyst provider. Raw notes remain immutable and searchable, while summaries can be expanded or invalidated without changing their source notes.

```sh
nexus memory add --scope project "The deployment requires the warren AWS profile."
nexus memory context --scope layered
nexus memory search "deployment|AWS"
nexus memory status
```

In local mode, project memories are shared by worktrees through their Git common directory. In remote mode, every clone configured with the same explicit `project_id` shares one project stream. Global memories apply across projects. Memory is never inferred automatically from prompts, transcripts, tool output, or assistant responses.

Nexus agent memory is inspired by Victor Taelin’s [OptMem](https://github.com/VictorTaelin/OptMem). Nexus uses an independent Rust/Diesel implementation and includes no OptMem source.

## Papercuts

Papercuts is the shared development-environment friction log. Agents can record flaky tests, slow checks, confusing setup, and similar recurring impediments; search the compact open list; upvote an existing match; resolve a fixed issue; or dump the complete timestamped registry for deeper inspection.

```sh
nexus papercuts list
nexus papercuts report "Flaky daemon startup test" --description "Times out under parallel load"
nexus papercuts upvote <papercut-id> --note "Reproduced in the MCP suite"
nexus papercuts remove <papercut-id> --resolution "Bounded the startup retry"
nexus papercuts dump | rg 'flaky|timeout'
nexus papercuts respond <review-id> --reporter-token <token> --decision approve --response "Same underlying failure"
```

Every report is a timestamped upvote. Exact normalized repeats collapse deterministically, resolved issues reopen when encountered again, and `dump` includes open, resolved, and consolidated records plus every timestamped upvote, resolution/reopen/consolidation transition, duplicate proposal, and reviewer outcome. Lifecycle transitions retain their actor provenance even when the current issue returns to the open state. A new report returns an opaque reporter token; Nexus stores only its hash and requires the token before accepting a later duplicate decision, so the reporter can answer even if its MCP process reconnects. With the optional analyst enabled, runtime requests an asynchronous review against at most 64 recently active open candidates. A possible semantic duplicate becomes a persisted question rather than a silent merge. The MCP surface exposes the same workflow as the single `nexus_papercuts` tool and the complete current-project dump as `nexus://papercuts`.

## SOS

SOS retains structured reports from agents that encounter a complex problem and need help, then asynchronously groups them into generated aggregate issues. Every source JSON remains immutable and auditable; an issue's importance and first/last arrival timestamps are derived from its linked reports. Resolving or deprecating an issue preserves its history, and a later recurrence reopens it.

```sh
nexus sos report "Release pipeline is wedged" \
  --type build_failure --environment ci-linux --version 1.7.0 \
  --model gpt-6 --trace 'step=publish; exit=137' \
  --metadata-json '{"job":"release","attempt":3}'
nexus sos list
nexus sos get <issue-id>
nexus sos dump | rg 'publish|release'
```

Authenticated agents can use the same workflow through `nexus_sos`, `nexus://sos`, or direct `POST /api/v1/sos`. The direct endpoint accepts exactly `message`, `type`, `environment`, `version`, `model`, `trace`, and an arbitrary bounded metadata object. Reports remain visible as untriaged if no model provider is available; bounded retry later resumes triage without delaying ingestion.

## Install

For same-machine development, one command from a Nexus source checkout installs the locked Rust build, bundled agent skills, MCP registrations, lifecycle hooks, the Pi terminal extension when Pi is present, and the migrated SQLite store. Source builds require Rust 1.88 or newer:

```sh
./scripts/install.sh
```

Then one command installs Nexus into any existing Git repository:

```sh
cd /path/to/repository && nexus install
```

`nexus install` writes the shared configuration to the Git common directory, so every worktree participates in the same project without repeated setup. Existing configuration is preserved. Nexus remains advisory: setup adds no enforcement or blocking mode. Codex and Claude use the installed MCP companion; it starts the local daemon when configured locally and forwards to the shared endpoint when configured remotely.

When setup uses a non-default configuration path, run `nexus --config /path/to/config.toml setup` or set `NEXUS_CONFIG` for that command. Nexus makes the path absolute and preserves it in the registered MCP, session-end commands, and installed Pi launch context, so later host processes keep using the selected remote endpoint regardless of their working directory. Registrations also bind each companion's agent identity for memory and papercut provenance. Direct `NEXUS_ENDPOINT`, `NEXUS_PROJECT_ID`, and `NEXUS_CLIENT_SECRET_FILE` overrides remain process-scoped; every agent host must inherit them if you choose those instead of a durable config file.

The bootstrap currently runs from a source checkout because this repository has no configured release remote. Once release hosting exists, that script is the stable boundary a remote one-liner can invoke.

## Shared PostgreSQL service

The repository includes a PostgreSQL-backed `nexus-server` image and a Compose stack. It exposes the service on host loopback port 7337 by default; set `NEXUS_PORT` to publish another host port and `NEXUS_BIND_ADDRESS` to select another host interface. Start it with an explicit high-entropy shared secret:

```sh
export POSTGRES_PASSWORD="$(openssl rand -hex 32)"
export NEXUS_SECRET="$(openssl rand -hex 32)"
export NEXUS_BIND_ADDRESS=127.0.0.1
export NEXUS_PORT=7337
docker compose up --build -d
curl --fail http://127.0.0.1:${NEXUS_PORT}/readyz
```

For the one-command path, use the startup helper instead. It automatically looks for the existing local SQLite database selected by `NEXUS_DATABASE_PATH`, `NEXUS_STATE_DIR`, or the platform's conventional Nexus state directory, then imports it while starting Compose:

```sh
./scripts/start-docker-service.sh
```

Pass a non-default database explicitly with `./scripts/start-docker-service.sh --sqlite /absolute/path/to/nexus.db`, or use `--no-import` for a deliberately fresh service. Compose mounts the source read-only into a network-isolated one-shot container, which takes a consistent SQLite online backup into a private named volume. The main image upgrades only that staged copy and transfers the logical records into PostgreSQL; neither container migrates or changes the source file. Sessions, claims, event identifiers and timestamps, usage, memory relationships and derived state, papercuts, and SOS source/aggregate history are retained. Backend-specific Flyway history is rebuilt normally rather than copied. Import accepts SQLite sources, consistent backups, and version-two and version-three record streams up to 4 GiB, with bounded batches and an 8 MiB per-record ceiling; it rejects unknown, newer, missing, or checksum-divergent source migrations rather than silently dropping state this binary does not understand.

For a maintenance-window migration, run `nexus-server --mode import --import-sqlite PATH` to import and exit without starting HTTP or model schedulers. `nexus-server --mode export` emits the PostgreSQL record stream for independent comparison before serving. Preserve an original archive before migration: historical V7/V8 schema normalizations run on the private copy and may replace oversized legacy values.

The import is atomic and restart-safe. A fresh PostgreSQL target records the snapshot digest after every row succeeds; restarting with that same snapshot reports a no-op. Nexus refuses a different snapshot after an import and refuses to merge into any nonempty PostgreSQL target that lacks the matching import marker. Remove `NEXUS_SQLITE_PATH` or use `--no-import` after a successful cutover if the old SQLite database will continue changing independently.

Direct Compose users can set `NEXUS_SQLITE_PATH=/absolute/path/to/nexus.db`. The staging container is the only process granted read-bypass capability, drops every other capability, has no network, and makes the private backup readable by the unprivileged service without changing the source permissions. Outside Compose, the equivalent server interface is `nexus-server --import-sqlite /absolute/path/to/nexus.db`; `NEXUS_IMPORT_SQLITE` supplies the same required path. The distributed image includes the required `nexus-sqlite-export` helper beside `nexus-server`.

If `NEXUS_SECRET` is omitted, `nexus-server` generates a 64-character secret and writes it once to the persistent service volume with mode `0600`. Nexus never prints the bearer value to logs. Copy it into a private client file or secret manager instead:

```sh
umask 077
docker compose exec -T nexus sh -c 'cat /var/lib/nexus/service.secret' > ./nexus-client.secret
chmod 600 ./nexus-client.secret
```

You can also launch the binary with `--secret`, `--secret-file`, or the matching `NEXUS_SECRET`/`NEXUS_SECRET_FILE` environment variables. After trimming boundary whitespace so ordinary newline-terminated secret files work, a secret must contain 32–4,096 visible ASCII characters with no embedded whitespace or control bytes. Server and companion clients apply the same content and file-metadata validation, and Nexus persists only the canonical trimmed bytes. Both accept an existing secret file only when it is a regular non-symlink file, is owned by the effective process user, and has no permissions beyond owner read/write (`0600`; a stricter subset such as `0400` is also valid). They reject unsafe metadata instead of silently changing it. If an explicit server secret disagrees with an existing secret file, startup fails rather than silently replacing the credential. A server bind-mounted secret therefore needs UID `10001` in the supplied image and mode `0400` or `0600`.

Configure every companion CLI or stdio MCP process that belongs to the same logical project with the same endpoint, project ID, and secret:

```toml
[client]
transport = "remote"
endpoint = "http://127.0.0.1:7337"
project_id = "organization/repository"
secret_file = "/absolute/path/to/nexus-client.secret"
```

`NEXUS_ENDPOINT`, `NEXUS_PROJECT_ID`, and `NEXUS_CLIENT_SECRET_FILE` provide equivalent non-secret client overrides. `NEXUS_SECRET` supplies the client bearer value directly and takes precedence over `client.secret_file`; the secret itself is deliberately excluded from typed configuration, `nexus config show`, and the configuration hash. The configured `nexus` CLI, generated hooks, `nexus mcp`, `nexus tui`, and Pi extension keep their existing commands and tools while routing through the remote service instead of starting a local daemon.

The remote `project_id` is the stable cross-environment identity. Two unrelated filesystem paths or Git clones share coordination, memory, usage, and papercuts only when they use the same value. Omitted read scopes default to this envelope project; read commands such as `sessions`, `claims`, `conflicts`, and `events` accept `--project-id` for explicit cross-project inspection by holders of the service-wide secret. Choose a durable, collision-resistant name and treat changing it as moving to a different Nexus project. The server never treats a client worktree path as a server-readable filesystem path.

The remote stdio companion keeps filesystem-dependent behavior on the machine that owns the checkout. After completed tool calls it derives bounded repository script and skill names locally, and every `coordination.reconcile_seconds` it snapshots Git-changed paths for active sessions and submits them through the authenticated API. It prunes expired sessions whenever activity arrives or a reconciliation begins and retains at most 1,024 recent sessions. Each Git identity or status process is limited to five seconds and 4 MiB from each output stream; Nexus kills and reaps a timeout or overflow, then fails open with fallback identity or no changed-path observation. Invalid optional usage evidence is discarded without dropping the completed-tool event. Changed paths are sent in rotating pages of at most 2,048 paths and 256 KiB of path text so a large checkout is eventually observed without an unbounded request. The supplemental evidence contains bounded repository-relative paths and normalized names—not file contents, diffs, environment values, command output, or credentials. Nexus validates its shape and binds completed-tool evidence to the lifecycle session and agent, but the service cannot independently prove a client's filesystem state. Treat every holder of the shared service secret as trusted across the endpoint: `project_id` is a namespace, not an authorization boundary. Direct HTTP MCP clients do not get this client-side filesystem observation unless they implement the companion behavior themselves.

Run exactly one `nexus-server` process against a given PostgreSQL database. The current service owns one synchronous Diesel connection and in-process consolidation/review scheduling; multiple replicas are not a supported high-availability topology. The Compose file intentionally starts one service replica.

The image contains Nexus and PostgreSQL client libraries, but it does not bundle or authenticate Codex or Claude. It runs as non-root UID `10001`; Compose drops all Linux capabilities and prevents privilege escalation. The Docker build context recursively excludes Git metadata, local environment files, SQLite databases and sidecars, socket artifacts, and secret files. Deterministic coordination, raw memory add/search/expand, usage analytics, and papercut report/list/dump/upvote/remove operations remain available without either provider.

`nexus-server` disables in-process model providers by default because a subprocess with the service UID can read the bearer-secret file and process state even when its inherited environment and working directory are scrubbed. `--allow-local-providers` is an explicit operator escape hatch, not a sandbox: use it only when the configured provider command crosses an externally enforced OS isolation boundary before invoking untrusted model tooling. Otherwise keep model-backed conflict analysis, duplicate review, and memory consolidation disabled on the service. Local-daemon provider behavior is unchanged.

The built-in listener serves plain HTTP/1.1 and uses one pre-shared bearer secret; bearer authentication authorizes requests but does not encrypt them, and it is not a public multi-tenant identity system. It admits at most 128 TCP connections, requires a complete request header within five seconds, caps headers at 32 KiB, and closes each connection after its response so unauthenticated idle or slow-header peers cannot accumulate server descriptors. Keep the default loopback bind for same-host use. Before crossing hosts, expose only a protected private interface or terminate TLS and apply network access controls at a trusted reverse proxy, then configure clients with the HTTPS endpoint. Never publish the development PostgreSQL password or expose plain HTTP to an untrusted interface.

## Run and inspect

```sh
nexus doctor
nexus daemon
```

In another terminal:

```sh
nexus status
nexus sessions
nexus claims
nexus conflicts
nexus tui
```

Nexus bundles the Pi terminal renderer and opens the dashboard without installing or starting Pi. It uses an existing Node 22.19+ runtime when available; otherwise it downloads and verifies its own pinned runtime on the first launch, then caches it for offline use. The command starts the daemon in local mode or uses the configured remote endpoint, and honors the current repository and `--config` settings. Press `q` or Ctrl-C to return to your shell.

The first screen shows project scope and health counts. Tab and Shift-Tab move through Coordination, SOS, Tools, Skills, Memory, and Papercuts. Coordination, SOS, and analytics remain read-only. In SOS, Enter drills from importance-ranked aggregates into retained reports and triage/lifecycle history. In Papercuts, `n` loads the next page of older open reports, Enter shows the full observation, and `r` refreshes or retries a failed read. In Memory, Enter expands a summary or inspects a raw note, `/` searches raw notes, `a` adds an explicitly scoped note, `f` confirms summary invalidation, `c` requests consolidation, and `r` refreshes. Raw notes cannot be edited or deleted.

For people who already use Pi, `nexus setup` also installs the `/nexus` extension inside Pi. Both entry points use the same dashboard components.

The MCP stdio server is the companion adapter used by installed hosts. It starts the daemon in local mode and forwards to `/api/v1/request` in remote mode. Remote connects are limited to three seconds, ordinary requests to fifteen seconds, and explicit analysis to the configured analyst timeout plus a five-second transport margin. Lifecycle hooks keep a stricter 2.5-second fail-open budget:

```sh
nexus mcp
```

Clients that support MCP Streamable HTTP can instead connect directly to `POST /mcp`. Every POST requires `Authorization: Bearer <secret>`, `Content-Type: application/json`, `X-Nexus-Project-ID`, and an `Accept` value containing both `application/json` and `text/event-stream`. Initialize without an `Mcp-Session-Id`; include a non-empty requested protocol version, an object-valued `capabilities` declaration, and `clientInfo` with non-empty `name` and `version`. Nexus reports its supported `2025-06-18` version in the response. The returned session is bound to the project plus optional `X-Nexus-Project-Root` and `X-Nexus-Agent` context. Before calling tools or resources, send `notifications/initialized` with that session ID and `MCP-Protocol-Version: 2025-06-18`, then retain both headers on later requests. JSON-RPC requests receive JSON responses and notifications receive `202 Accepted`. Authenticated `DELETE /mcp` with the protocol and session headers closes the session (`204`, or `404` if it is already absent); authenticated `GET /mcp` with the protocol header remains intentionally unsupported. Missing or unsupported protocol versions on subsequent MCP requests return `400`. Idle sessions expire after 24 hours and the service bounds its session registry. Requests carrying an `Origin` header are rejected unless that exact origin was passed to `nexus-server --allow-origin` (repeat the flag for multiple origins).

Authentication and Origin checks run before body collection. Authenticated `/api/v1/request` and `/mcp` bodies are capped at 1 MiB and must arrive within five seconds; the listener admits at most 64 concurrent protected requests and waits at most one second for capacity. Readiness database work is independently single-flight.

`GET /healthz` is a process liveness check and `GET /readyz` verifies that the service can reach its store. PostgreSQL readiness always uses an independent probe connection, so a slow or canceled health query cannot seize the application store and short legitimate contention does not produce a false restart. A continuously held application-store guard becomes unhealthy after 30 seconds, and a poisoned application-store mutex fails immediately, rather than either condition being hidden by the independent probe; this lets the post-ready healthcheck restart an unusable service. The server clears its readiness marker on startup and writes it only after database initialization, migrations, and listener binding succeed. Before that marker exists, twelve consecutive failed health checks send `SIGKILL` to PID 1 because a container init process can ignore default-action `SIGTERM` before Nexus installs its handler; this lets Compose recover from a database connect or migration that never returns. After the marker exists, three consecutive genuine readiness failures send handled `SIGTERM` so Compose establishes a fresh PostgreSQL connection. The counters are separate and clear on success; the production smoke and shell regressions exercise post-ready recovery and the startup watchdog. The typed companion API at `POST /api/v1/request` also requires the bearer secret and carries `project_id` outside the feature request payload.

The Tools tab starts with Activities, which describes work such as checking CI, searching code, or inspecting version control. Keys `1`–`4` select Activities, Capabilities, Opportunities, and Legacy. Capabilities retain exact tool and script identities and include unused resources when inventory is available. Opportunities show recurring inline/CLI work and same-category capabilities offered in the observed sessions. Enter opens implementations and provenance; `n`/`p` pages results. Availability that a host does not expose stays unknown. Older shell observations remain in Legacy because their arguments were never stored.

Opaque work is classified asynchronously by the local companion using the configured analyst provider/model, with tools and hooks disabled. Redacted bounded inputs stay in memory; analytics persists labels and fingerprints. `[analytics]` controls enablement, timeout (30 seconds), and the shared daily attempt budget (128). See [the analytics contract](docs/features/usage-analytics.md) for counting, coverage, privacy, and category maintenance.

Nexus supports macOS and Linux. If a macOS machine has full Xcode selected but its license is pending, builds can use the separately installed Command Line Tools without changing system-wide state:

```sh
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo build
```

## Agent integrations

`./scripts/install.sh` performs these integrations automatically. The commands below are inspection tools for development and troubleshooting.

Print current MCP registration and lifecycle-hook JSON for either host:

```sh
nexus integrate codex
nexus integrate claude
```

The output includes the host's MCP registration command and a `settings_fragment` to merge into its hooks file. Nexus uses MCP hooks for user prompts, pre-tool inspection, successful tool completion, and Claude tool failures. Both hosts use a command hook for session end because MCP context is unavailable during teardown.

## Configuration

Nexus merges configuration in this order:

1. built-in defaults
2. the user file at `~/.config/nexus/config.toml`, or `NEXUS_CONFIG`
3. `<git-common-dir>/nexus.toml`
4. supported environment overrides
5. explicit CLI arguments

Copy [`config.example.toml`](config.example.toml) to begin. Useful commands:

```sh
nexus config check
nexus config show
```

`config show` includes the resolved configuration, loaded source files, and a stable configuration hash. Unknown properties and invalid values are rejected rather than silently ignored. Credentials do not belong in this file. Supply a dedicated Claude API key as `NEXUS_CLAUDE_KEY` in the process environment at startup. Nexus maps it to `ANTHROPIC_API_KEY` only for Claude subprocesses, overriding ambient API/OAuth tokens; it is never serialized into configuration or sent to Codex.

`[client]` selects `local` or `remote` transport. A remote client requires an HTTP(S) endpoint and explicit project ID; its optional `secret_file` points at the credential without embedding it in configuration. Server-only `NEXUS_DATABASE_URL` and `NEXUS_SECRET_FILE` are separate from the client overrides.

Agent autonomy is not a configurable property. There is intentionally no enforcement or blocking mode.

The `[ui]` properties select the terminal dashboards' polling interval and bounded event/record windows. Nexus rejects zero limits.

## Optional conflict analyst

The deterministic classifier is always the source of conflict severity. An optional analyst can add a conflict summary and suggested resolutions, and can review newly reported papercuts for possible semantic duplicates, using an existing local Codex or Claude installation:

```toml
[analyst]
enabled = true
provider = "claude" # or "codex"

[analyst.claude]
command = "claude"
model = "your-selected-model"
```

Request conflict analysis explicitly; it never runs in the pre-tool path:

```sh
nexus analyze <conflict-id>
```

Codex runs with a read-only sandbox and Claude runs in plan permission mode. Results are advisory and are stored with provider, model, timestamp, and resolved configuration hash provenance. Papercut duplicate review runs only after the report is durable and cannot silently merge issues: a proposed match waits for a response carrying the opaque token returned with the original report. One worker serializes review execution behind a bounded 32-item queue and compares against at most 64 recently active open candidates. Explicit conflict analysis and papercut review do not silently fall back to a different provider; background memory consolidation has its separately documented one-provider fallback. Remote `nexus-server` deployments keep all provider execution disabled unless the operator supplies `--allow-local-providers` together with an externally isolated provider command.

## Coordination model

- The event log is append-only history.
- Sessions, claims, conflicts, and advisories are queryable projections.
- Claims are renewable, expiring signals—not locks.
- Local Git common-directory identity groups worktrees from the same repository; remote clients use their explicit shared project ID.
- Tool hooks claim known paths before execution.
- Background Git reconciliation observes changes made through unknown or bypassed tools. The local daemon reads its worktrees directly; a remote stdio companion reads each active client checkout periodically and submits bounded authenticated path evidence to the service.
- Exact hunk and destructive overlaps are critical advisories; unknown-range path overlaps are warnings; known disjoint hunks are informational.
- Successful execution after an advisory records an `advisory_ignored` event for later context, without judging or reversing the agent's choice.

## Code architecture

The source tree follows capability boundaries rather than a flat layer list:

- `coordination/` owns the typed service API, domain states, classification, workspace observation, and papercut workflow orchestration.
- `persistence/` owns Diesel models, schema, migrations, and projection queries.
- `runtime/` owns the configured client, Unix-socket daemon, authenticated HTTP service, and MCP stdio/Streamable HTTP adapters.
- `agents/` owns optional conflict and papercut reviewer processes plus host integration generation.
- `papercuts/` owns the durable registry's domain states and timestamped observation types.
- `pi-extension/` owns the terminal-native progressive-disclosure view. Coordination, usage, and memory use one correlated local MCP child, which follows the configured local or remote client transport.

JSON is decoded at runtime boundaries into the closed `ServiceRequest` enum. Open-ended tool input remains isolated in the named `ToolPayload` boundary type; coordination and persistence APIs use explicit records and enums. Behavioral choices such as record scope, conflict scope, completion state, and release target are enums rather than boolean arguments.

Explore the implementation by feature before reading it file by file:

```sh
python3 scripts/feature_map.py --root . --list
python3 scripts/feature_map.py --root . --feature coordination
```

Every code file links to a long-form specification under `docs/features/`. Architecture lint verifies those links, declared entry points, intentional dynamic-data boundaries, and Python type annotations:

```sh
python3 scripts/feature_map.py --root . --lint
```

The feature-aware complexity wrapper uses `big-code-analysis` to report review leads by file, symbol, class, or feature. The limits in `bca.toml` and repetition policy in `complexity.toml` are coarse regression gates, not instructions to split a deep function or merge implementations with different invariants. See the [complexity-analysis feature specification](docs/features/complexity-analysis.md) for the metric contract and interpretation rules.

```sh
python3 scripts/complexity_analysis.py --feature coordination
python3 scripts/complexity_analysis.py --check
```

SQLite and PostgreSQL persistence use [Diesel](https://github.com/diesel-rs/diesel). The default `nexus` binary compiles the SQLite backend for local operation; the container builds `nexus-server` with the PostgreSQL backend. Both use the same typed application queries and mutations. Raw SQL query strings are not used by application code; SQL appears only in the backend-specific forward-only schema files under `migrations/` and `postgres_migrations/`.

Every `Store` instantiation runs the selected backend's embedded, ordered [flyway-rs](https://github.com/tdcare/flyway-rs) migrations before exposing the connection. A fresh database receives the complete schema; an existing database receives only versions absent from the typed `flyway_migrations` history table. Released `V<version>_<description>.sql` files are immutable—later schema changes add a new version in both backend directories.

## Cross-model commit review

The local pre-commit hook runs two read-only reviews concurrently after deterministic checks: `$code-architect` and `$code-deletion`. The configured reviewer must differ from the implementation provider. Review text and provider errors are advisory output for the implementation agent; they never become a permission or commit-denial decision.

Configure the implementation/review pairing and model selections under `[review]` in [`config.example.toml`](config.example.toml). The default assumes Codex implementation and Claude review, with `xhigh` reasoning and a ten-minute timeout. Run the review directly with:

```sh
cargo run --bin nexus -- review-commit
```

## Development checks

Install the pinned complexity analyzer, then run the same validation entry point used by CI:

```sh
uv tool install big-code-analysis-cli==2.2.0
./scripts/check.sh
```

Pi extension development also requires Node 24 and uses the locked `pi-extension/package-lock.json`; `scripts/check.sh` installs that tree, type-checks the TypeScript sources, and runs the terminal UI tests.

If `uv` is unavailable, `python3 -m pip install big-code-analysis-cli==2.2.0` installs the same pinned CLI.

Enable the repository's tracked pre-commit hook for the current checkout with:

```sh
./scripts/install-hooks.sh
```

The hook invokes `scripts/check.sh`, which removes live service/provider key exports from the test process, then runs feature-map and complexity lint, both Python tooling test modules, the Docker startup-watchdog regression, Rust formatting, SQLite and PostgreSQL server Clippy checks, and the complete default-backend Rust test suite. It then runs both advisory cross-model review skills in parallel against the staged diff. CI additionally runs the PostgreSQL integration test against PostgreSQL 17 and exercises the production Compose service end to end, including generated-secret authentication and Nexus/PostgreSQL restart persistence.

## Tests

```sh
cargo test --all-targets
```

The suite includes unit tests for extraction and classification plus process-level tests that launch a real daemon and MCP stdio server. Those tests call user-prompt, pre-tool, post-tool, failure, resolution, analyst, papercut, and session-stop surfaces; inspect the Diesel-backed SQLite projections; exercise Edit, Write, apply-patch, shell, unknown-tool, malformed-input, and unavailable-daemon cases; and verify background Git reconciliation and reviewer dispatch. Remote runtime tests cover bearer authentication, cross-environment project binding, bounded client reconciliation and derived usage evidence, Streamable HTTP initialization/session deletion, and generated-secret persistence. CI's PostgreSQL test exercises coordination, memory, and papercut semantics on the service backend.

The fresh-install worktree test is intentionally ignored because it consumes real Codex and Claude Opus 5 resources and requires both CLIs to be authenticated. Run it explicitly with:

```sh
cargo test --test install_e2e fresh_install_coordinates_real_agents_across_worktrees -- --ignored --exact
```

It installs Nexus into a disposable prefix, initializes a repository, creates separate Codex and Claude worktrees, makes both models call the Nexus MCP server for overlapping hunks, and requires the stored result to be a critical conflict.
