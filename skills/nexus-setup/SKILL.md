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

## Keep an installation current

```sh
nexus --version
nexus update --check
nexus update
```

`update` compares the full installed source revision with current `main`, then
installs its verified versioned release without a Nexus checkout, `git pull`, or
Rust compilation. A release still being built is reported as `release_pending`;
retry after publication. Use `--force` only when intentionally replacing a dirty,
ahead, divergent, or unknown source build with main. Normal updates preserve the
existing executable location and refresh bundled skills, hooks, and extensions
through `setup --refresh` with the retained explicit configuration path. This refresh also supports headless machines with no installed agent host.

A running managed local daemon drains and restarts with its exact resolved
configuration, socket, lock, database, and working directory. An inactive daemon
stays inactive. Managed local Docker Compose deployments retain their project,
resolved environment, secrets, health checks, networks, and volumes while only
the Nexus image is replaced. The Docker server's platform determines the image,
including when the client runs on macOS. The Compose startup script records
management automatically; legacy known stacks can be adopted from their exact
endpoint and container labels. If needed, supply `nexus update --compose-file
/path/to/docker-compose.yml`; the path must match the running service's labels.
The resolved private management snapshot supports later updates after that source
checkout is gone. Source or runtime drift that prevents proving the running settings is refused before replacement; restore matching configuration or explicitly adopt the proven running deployment.

Read the report's `service` and `notes` fields. An external or unverifiable legacy
service is not silently restarted; its owner must update it separately. Restart
or reconnect existing agent MCP companions after update so those processes load
the new binary. Do not restart the user's agent application or mutate database
volumes to force an upgrade. Failed managed transitions restore the previous
binary and attempt service recovery; data and migrations are not rolled back.

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

## Onboard another independent clone

From the new clone run:

```sh
nexus install --share-project-with /path/to/existing/checkout
nexus config show
nexus doctor
```

This atomically copies only the existing checkout's resolved remote project ID.
Both checkouts must use the same remote service; credentials and other settings
remain local to each checkout. Worktrees already share their common Git config.
Inspect both resolved configurations, since environment overrides still win.
Changing a binding retains old history in its previous namespace. Reconnect
already-running companions after installation or a binding change.

For cross-checkout tools, verify the actual `workdir`/`cwd` or file path, not just
the host's startup directory. The companion selects configuration for that target
checkout and native session end cleans up its recorded visits. Missing coverage
is advisory and visible; it never blocks the workload.

Claude's SessionStart command adapter exports its exact native identity through
`CLAUDE_ENV_FILE` when supplied by the host. This enables runner-side `work
context` resolution without guessing by checkout. Existing sessions need a new
supported SessionStart boundary or an explicit identity handoff; refreshing the
binary cannot retroactively change a running shell's environment.
