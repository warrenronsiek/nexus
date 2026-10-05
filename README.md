# Nexus

Nexus is a local coordination service for coding agents. It observes tool activity, records advisory path claims, detects overlapping work, and gives agents context without controlling their execution.

The invariant is structural: every lifecycle-hook response permits the original tool call. Nexus emits no deny, block, approval, or input-rewrite decision.

## Agent memory

Nexus keeps deliberately authored memories in its local SQLite store and supplies bounded, clearly delimited historical context once per agent session. Agents only decide which durable notes to add; Nexus builds disposable hierarchical summaries in the background with the configured Codex or Claude analyst provider. Raw notes remain immutable and searchable, while summaries can be expanded or invalidated without changing their source notes.

```sh
nexus memory add --scope project "The deployment requires the warren AWS profile."
nexus memory context --scope layered
nexus memory search "deployment|AWS"
nexus memory status
```

Project memories are shared by worktrees through their Git common directory. Global memories apply across projects. Memory is never inferred automatically from prompts, transcripts, tool output, or assistant responses.

Nexus agent memory is inspired by Victor Taelin’s [OptMem](https://github.com/VictorTaelin/OptMem). Nexus uses an independent Rust/SQLite implementation and includes no OptMem source.

## Install

From a Nexus source checkout, one command installs the locked Rust build, bundled agent skills, MCP registrations, lifecycle hooks, the Pi terminal extension when Pi is present, and the migrated SQLite store:

```sh
./scripts/install.sh
```

Then one command installs Nexus into any existing Git repository:

```sh
cd /path/to/repository && nexus install
```

`nexus install` writes the shared configuration to the Git common directory, so every worktree participates in the same project without repeated setup. Existing configuration is preserved. Nexus remains advisory: setup adds no enforcement or blocking mode. Codex and Claude start the local daemon through MCP when they need it.

The bootstrap currently runs from a source checkout because this repository has no configured release remote. Once release hosting exists, that script is the stable boundary a remote one-liner can invoke.

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
nexus ui
```

`nexus ui` opens a local, read-only operations dashboard with machine-wide activity, repository filtering, conflict/session/claim panels, a one-hour activity chart, and progressively disclosed record details. Use `nexus ui --launch print` to start it and print the URL without opening a browser. The dashboard is optional: a UI bind or browser-launch failure never stops Unix-socket coordination.

With Pi installed, the same setup command installs a terminal-native view. Start Pi and run:

```text
/nexus
```

The first screen shows project scope and health counts. Tab and Shift-Tab move through Coordination, Tools, Skills, and Memory. Coordination and analytics remain read-only. In Memory, Enter expands a summary or inspects a raw note, `/` searches raw notes, `a` adds an explicitly scoped note, `f` confirms summary invalidation, `c` requests consolidation, and `r` refreshes. Raw notes cannot be edited or deleted.

The MCP stdio server starts the daemon automatically when needed:

```sh
nexus mcp
```

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

`config show` includes the resolved configuration, loaded source files, and a stable configuration hash. Unknown properties and invalid values are rejected rather than silently ignored. Credentials do not belong in this file.

Agent autonomy is not a configurable property. There is intentionally no enforcement or blocking mode.

The `[ui]` properties select dashboard enablement, the loopback bind address, polling interval, and bounded event/record windows. Nexus rejects non-loopback addresses and zero limits; remote access and authentication are intentionally out of scope.

## Optional conflict analyst

The deterministic classifier is always the source of conflict severity. An optional analyst can add a summary and suggested resolutions using an existing local Codex or Claude installation:

```toml
[analyst]
enabled = true
provider = "claude" # or "codex"

[analyst.claude]
command = "claude"
model = "your-selected-model"
```

Request analysis explicitly; it never runs in the pre-tool path:

```sh
nexus analyze <conflict-id>
```

Codex runs with a read-only sandbox and Claude runs in plan permission mode. Results are advisory and are stored with provider, model, timestamp, and resolved configuration hash provenance. Explicit conflict analysis does not silently fall back to a different provider; background memory consolidation has its separately documented one-provider fallback.

## Coordination model

- The event log is append-only history.
- Sessions, claims, conflicts, and advisories are queryable projections.
- Claims are renewable, expiring signals—not locks.
- Git common-directory identity groups worktrees from the same repository.
- Tool hooks claim known paths before execution.
- Background Git reconciliation observes changes made through unknown or bypassed tools.
- Exact hunk and destructive overlaps are critical advisories; unknown-range path overlaps are warnings; known disjoint hunks are informational.
- Successful execution after an advisory records an `advisory_ignored` event for later context, without judging or reversing the agent's choice.

## Code architecture

The source tree follows capability boundaries rather than a flat layer list:

- `coordination/` owns the typed service API, domain states, classification, and workspace observation.
- `persistence/` owns Diesel models, schema, migrations, and projection queries.
- `runtime/` owns the Unix-socket daemon and MCP protocol adapters.
- `runtime/web.rs` owns the read-only local HTTP adapter and embedded frontend.
- `agents/` owns optional analyst processes and host integration generation.
- `ui/` owns the Elm application and the narrow typed D3 adapter; compiled `ui/dist` assets are tracked and embedded in the Rust binary.
- `pi-extension/` owns the terminal-native progressive-disclosure view. Coordination and usage stay on the read-only HTTP API; memory content uses the correlated local MCP transport and never enters a browser endpoint.

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

SQLite persistence uses [Diesel](https://github.com/diesel-rs/diesel). Application queries and mutations use Diesel's typed query DSL; raw SQL query strings are not used. SQL appears only in the forward-only schema files under `migrations/`.

Every `Store` instantiation runs embedded, ordered [flyway-rs](https://github.com/tdcare/flyway-rs) migrations before exposing the connection. A fresh database receives the complete schema; an existing database receives only versions absent from the typed `flyway_migrations` history table. Released `V<version>_<description>.sql` files are immutable—later schema changes add a new version.

## Cross-model commit review

The local pre-commit hook runs two read-only reviews concurrently after deterministic checks: `$code-architect` and `$code-deletion`. The configured reviewer must differ from the implementation provider. Review text and provider errors are advisory output for the implementation agent; they never become a permission or commit-denial decision.

Configure the implementation/review pairing and model selections under `[review]` in [`config.example.toml`](config.example.toml). The default assumes Codex implementation and Claude review, with `xhigh` reasoning and a ten-minute timeout. Run the review directly with:

```sh
cargo run -- review-commit
```

## Development checks

Install the pinned complexity analyzer, then run the same validation entry point used by CI:

```sh
uv tool install big-code-analysis-cli==2.2.0
./scripts/check.sh
```

UI development also requires Node 24 and uses the locked `ui/package-lock.json` and `pi-extension/package-lock.json`; `scripts/check.sh` installs both trees, runs Elm and TypeScript tests, type-checks, and verifies the committed browser bundle is reproducible.

If `uv` is unavailable, `python3 -m pip install big-code-analysis-cli==2.2.0` installs the same pinned CLI.

Enable the repository's tracked pre-commit hook for the current checkout with:

```sh
./scripts/install-hooks.sh
```

The hook invokes `scripts/check.sh`, which runs feature-map and complexity lint, both Python tooling test modules, Rust formatting and Clippy checks, and the complete Rust test suite. It then runs both advisory cross-model review skills in parallel against the staged diff.

## Tests

```sh
cargo test --all-targets
```

The suite includes unit tests for extraction and classification plus process-level tests that launch a real daemon and MCP stdio server. Those tests call user-prompt, pre-tool, post-tool, failure, resolution, analyst, and session-stop surfaces; inspect the Diesel-backed SQLite projections; exercise Edit, Write, apply-patch, shell, unknown-tool, malformed-input, and unavailable-daemon cases; and verify background Git reconciliation.

The fresh-install worktree test is intentionally ignored because it consumes real Codex and Claude Opus 5 resources and requires both CLIs to be authenticated. Run it explicitly with:

```sh
cargo test --test install_e2e fresh_install_coordinates_real_agents_across_worktrees -- --ignored --exact
```

It installs Nexus into a disposable prefix, initializes a repository, creates separate Codex and Claude worktrees, makes both models call the Nexus MCP server for overlapping hunks, and requires the stored result to be a critical conflict.
