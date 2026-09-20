# Nexus

Nexus is a local coordination service for coding agents. It observes tool activity, records advisory path claims, detects overlapping work, and gives agents context without controlling their execution.

The invariant is structural: every lifecycle-hook response permits the original tool call. Nexus emits no deny, block, approval, or input-rewrite decision.

## Build and run

```sh
cargo build
./target/debug/nexus doctor
./target/debug/nexus daemon
```

In another terminal:

```sh
./target/debug/nexus status
./target/debug/nexus sessions
./target/debug/nexus claims
./target/debug/nexus conflicts
```

The MCP stdio server starts the daemon automatically when needed:

```sh
./target/debug/nexus mcp
```

Nexus supports macOS and Linux. If a macOS machine has full Xcode selected but its license is pending, builds can use the separately installed Command Line Tools without changing system-wide state:

```sh
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo build
```

## Agent integrations

Print current MCP registration and lifecycle-hook JSON for either host:

```sh
./target/debug/nexus integrate codex
./target/debug/nexus integrate claude
```

The output includes the host's MCP registration command and a `settings_fragment` to merge into its hooks file. Nexus hooks cover user prompts, pre-tool inspection, successful tool completion, and—where supported—tool failure and session end.

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

Codex runs with a read-only sandbox and Claude runs in plan permission mode. Results are advisory and are stored with provider, model, timestamp, and resolved configuration hash provenance. Nexus does not silently fall back to a different provider.

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
- `agents/` owns optional analyst processes and host integration generation.

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

Configure the implementation/review pairing and model selections under `[review]` in [`config.example.toml`](config.example.toml). The default assumes Codex implementation and Claude review. Run the review directly with:

```sh
cargo run -- review-commit
```

## Development checks

Install the pinned complexity analyzer, then run the same validation entry point used by CI:

```sh
uv tool install big-code-analysis-cli==2.2.0
./scripts/check.sh
```

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
