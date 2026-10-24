---
name: nexus-setup
description: Connect an existing repository or agent host to Nexus, configure local or shared service transport, and diagnose CLI, MCP, and lifecycle-hook integration.
---

# Connect existing work to Nexus

Nexus adds advisory coordination, durable history, memory, and observed analytics
to the caller's existing tools. Keep the caller's runner, repository conventions,
and delivery policy. Setup connects those tools; it does not turn Nexus into a
task scheduler or make telemetry a condition of executing work.

## Install and verify

With the installed `nexus` executable on `PATH`:

```sh
nexus setup
cd /path/to/your/repository
nexus install
nexus config check
nexus doctor
```

`setup` installs bundled skills into both Codex and Claude skill directories,
registers the `nexus` MCP companion and merges lifecycle hooks for installed
Codex/Claude hosts, and installs the Pi extension when Pi is present. It requires
at least one of Codex or Claude. Rerunning refreshes bundled resources and
preserves unrelated host settings and adjacent user files. Use `CODEX_HOME` or
`CLAUDE_CONFIG_DIR` when those hosts use nondefault configuration directories.

`install` creates `nexus.toml` in Git's common directory, shared by worktrees,
without overwriting an existing file. Both commands initialize SQLite only for
local transport. Installed use needs no Nexus source checkout. If the executable
is absent, use the project's documented bootstrap/release installation rather
than inventing a package-manager command.

`config check` validates the resolved configuration. `config show` returns its
non-secret values, sources, and stable hash. `doctor` checks selected transport
and storage prerequisites: local success verifies the database, not every host
hook; remote success verifies service readiness. Use `status` and a representative
host event to verify the entire connection.

## Select one state authority

Local transport is the default: a stdio companion reaches or starts the
machine-local Unix-socket daemon backed by SQLite. Shared deployments use the
same installed CLI and `nexus mcp` companion with remote configuration. They do
not fall back to a separate local store on failure.

Read [remote and host integration](references/integration.md) when selecting a
shared service, configuring credentials, or implementing a custom host adapter.
Keep the configured project ID stable across clones that should share history;
it is a namespace within the service trust domain, not a separate credential or
access-control boundary.

## Resolve configuration before changing it

The actual merge order is defaults, the selected user file, repository
Git-common-directory `nexus.toml`, then supported environment overrides.
`--config PATH` or `NEXUS_CONFIG` selects that user-file layer instead of the
platform default `nexus/config.toml`; it does not override later project or
environment values. Unknown properties and invalid final values are rejected.

Run configuration inspection from the same checkout and environment as the
caller. Setup retains an explicit config path as an absolute path in generated
host commands, including session-end hooks and Pi launch context. This keeps
hooks pointed at the same configuration after their working directory changes.
The path can still be layered with repository configuration at invocation time.

## Fit the caller's integration

Prefer `nexus setup` over hand-maintained registration JSON. `nexus integrate
codex` and `nexus integrate claude` print registration and hook fragments for
managed installations; merge their hooks instead of replacing unrelated ones.
The generated hooks observe prompt, tool-start/completion/failure where the host
supports them, and session end. Session end uses a command hook because MCP may
already be gone. Advisories provide context and never approve, deny, or rewrite
the caller's operation.

For a custom MCP client, discover the installed tool schemas and resources.
Lifecycle tools observe the host's actual events; explicit feature tools record
caller intent or inspect state. Keep those lifecycles separate. A new MCP process
does not by itself prove that a host task started or finished.

Use the feature skill matching the next integration:
[coordination](../nexus-coordination/SKILL.md) for overlap advisories,
[usage analytics](../nexus-usage-analytics/SKILL.md) for host inventory and semantic
usage, [work history](../nexus-work-history/SKILL.md) for task boundaries and
scripts, [eval history](../nexus-eval-history/SKILL.md) for caller-run results and
artifacts, and [observability](../nexus-observability/SKILL.md) for terminal views.
Use [Nexus development](../nexus-development/SKILL.md) when modifying Nexus itself.
Verify one real workflow,
its scope and attribution, plus behavior when Nexus is unavailable; missing
optional capture must not change or repeat the workload.
