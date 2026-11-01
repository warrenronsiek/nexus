---
feature: installation
---

# Installation

## What this feature does

Installation connects existing agent hosts, repositories, and people to Nexus. Machine setup installs the feature and review skills, registers the MCP companion for supported hosts, merges advisory lifecycle hooks, installs headless Pi capture when present, and initializes SQLite only for local transport. Repository installation creates shared Git-common-directory configuration without replacing existing decisions. The read-only Elm/D3 browser application is embedded in both native and service executables; users need no Node runtime or separate frontend service.

## Onboarding and browser access

```sh
./scripts/install.sh
cd /path/to/repository
nexus install
nexus setup --web-user viewer --web-password-file /absolute/private/password-file
nexus web
```

Open `http://127.0.0.1:7338` and sign in with the configured viewer credentials.
The password file must be an owned regular file with permissions no broader than
`0600`; `--web-password-file -` reads standard input instead. Passwords are
12–1024 UTF-8 bytes; usernames are 1–64 ASCII letters, digits, dots, underscores,
or hyphens. Do not put passwords in command arguments, environment variables,
TOML, or committed scripts. Setup stores only a salted Argon2id verifier in a
private file under the platform's local Nexus data directory. Its JSON report
contains the username and credential-file path, never the password or verifier.
`--web-credentials-file PATH` selects another output file. Refresh without
credential flags preserves the existing verifier. Supplying viewer credentials
also supports headless setup; ordinary agent setup still requires a supported
host. Reconfigure credentials, then restart the viewer to rotate access and
invalidate its in-memory sessions.

`nexus web` uses the normal resolved client configuration: local SQLite remains
owned by the daemon, which the viewer starts when needed; remote mode forwards
typed reads to the selected service without starting a local daemon.
`--credentials-file PATH` selects its verifier and `--listen HOST:PORT` changes
its loopback default. `--public-origin https://nexus.example` enables an exact
external HTTPS origin behind a TLS reverse proxy. Plain HTTP browser access is
limited to loopback. All data operations are reads, including memory context
inspection; agent recording commands remain separate.

For Compose, provision the same verifier into the service's persistent volume
before startup:

```sh
./scripts/start-docker-service.sh --web-credentials /absolute/path/from/setup/web-credentials.json
```

The startup helper streams that private file into the volume as the unprivileged
service user. It never bind-mounts a host-owned verifier or passes it through
Compose environment values. Omit the flag on subsequent starts to preserve it.
The service exposes the browser at `http://127.0.0.1:7337` alongside its existing
agent API. Set `NEXUS_WEB_ORIGIN` to the exact external HTTPS origin when using a
reverse proxy. The browser account and opaque session cookie are independent of
the agent bearer secret. Project selection is a filter within one service-wide
viewer trust domain, not a per-project access-control boundary.

With no import arguments, `scripts/start-docker-service.sh` discovers the
configured or conventional SQLite database and imports it into the Compose
stack. `--sqlite PATH` selects another source; `--no-import` starts fresh. The
network-isolated one-shot staging container mounts the source read-only and
publishes a backup in a private volume. Persistence owns import validation;
installation owns discovery and Compose instantiation.

## Installation flow

1. Bootstrap installs the locked Rust binary; subsequent upgrades use verified releases.
2. `nexus setup` installs the complete skill library, host MCP registrations,
   advisory hooks, and headless Pi capture while retaining unrelated files.
3. An explicit `--config` or `NEXUS_CONFIG` is resolved to an absolute path and
   preserved as argv in host registrations, command hooks, and Pi launch context.
4. Local setup applies embedded migrations. Remote setup creates no client-side
   database; `nexus doctor` separately verifies authenticated reachability.
5. `nexus install` preserves or creates configuration in Git's common directory,
   sharing identity across worktrees. Remote clones use an explicit stable project ID.
6. Optional viewer credential setup and `nexus web`, or Compose provisioning,
   expose the same authenticated read-only application.

## Implementation details

The installed `nexus` skill maps all seventeen feature IDs to focused guides:
`nexus-setup` covers installation, configuration, and runtime;
`nexus-coordination` covers coordination and analyst;
`nexus-agent-memory`, `nexus-papercuts`, `nexus-code-concerns`, `nexus-sos`,
`nexus-usage-analytics`, and `nexus-observability` cover their respective
capabilities; `nexus-development` covers architecture tooling, complexity,
commit review, and persistence. Existing eval and work-history guides retain
their contracts. References and cross-skill links resolve within the installed
library; source-development commands explicitly require a Nexus checkout.
Installation tests compare the complete source skill resource tree with both
host installations so a new reference cannot be omitted from the bundle.

Setup also installs `nexus-work-history` for Codex and Claude, including its
integration and host-adapter references and the canonical `work-capture.sh` and
`nexus-work-github.py` sources under the installed skill's `scripts/` directory.
The helpers remain maintained in the repository's top-level `scripts/`; setup
embeds those exact files rather than maintaining another implementation. The
installed skill therefore works outside a Nexus checkout. Its guidance preserves
caller-owned execution, nested identity, qualified timing, metadata privacy, and
fail-open hooks. Isolated installation tests exercise the bundled shell helper's
exactly-once fallback, preserved output/exit status, and runnable Python adapter.

`setup_machine` and `install_repository` in `src/installation.rs` are the onboarding entry points. `src/installation_assets.rs` contains the embedded skill and Pi module inventories; setup installs all eighteen named skills and their references. Setup is idempotent: skill and Pi-extension contents are replaced only when different, the named MCP registration is refreshed, and generated Nexus hook entries replace only prior Nexus entries. Repository installation never overwrites an existing `nexus.toml`. Both entry points load typed client configuration before touching persistence: local mode opens and migrates SQLite, while remote mode reports remote storage and creates no local database. The generated host integration is transport-neutral because `nexus mcp` and the session-end command both use the centralized configured client. An explicit setup configuration path is retained in both installed commands and Pi's generated launch context, and every MCP argv names its host so process-bound memory and papercut provenance remains accurate.

The source bootstrap entry point is `./scripts/install.sh`. Source builds require Rust 1.88 or newer. The script installs with Cargo's locked dependency graph into `NEXUS_INSTALL_ROOT`, `CARGO_HOME`, or the normal `~/.cargo` prefix in that order, then calls the installed binary's `nexus setup` command. Subsequent updates use verified release assets through `nexus update`; they require neither this checkout nor a Rust build.

`tests/installation.rs` exercises both commands as child processes with isolated homes and provider binaries. The ignored real-agent test exercises the packaged install plus two worktrees using authenticated Codex and Claude clients; it must be requested explicitly because it consumes model resources and requires external credentials.

Pi installation embeds only capture, inventory, automatic memory context, launch
context, and bounded MCP transport modules. It removes the known retired Nexus
UI files on refresh while retaining user-created helpers and other extensions.
Installation tests resolve the complete import closure in an isolated home.

`web/dist` is the tracked, deterministic Vite/Elm/D3 build. The canonical check
installs locked frontend dependencies, runs TypeScript/Elm and chart tests,
rebuilds it, and rejects drift. Cargo, bootstrap, Docker, and per-platform release
builds embed that exact validated asset whitelist; they do not fetch Node or Elm
at runtime or rebuild assets independently. Generated caches and dependencies
are excluded from the Docker context. Setup regression tests use isolated homes
and private files; browser authentication and local/remote reads have separate
HTTP integration tests.

## Versioned updates

`nexus --version` identifies the human package version, full source commit, build
platform, and whether the source checkout was modified. `nexus update --check`
compares that identity with the repository's exact `main` reference and reports
`current`, `available`, `release_pending`, or `requires_force`. Check mode does
not download executable/image assets, write management files, invoke setup, or
contact the selected service. A dirty, ahead, divergent, or unknown source build
requires `--force` before replacement. The updater never fetches into or modifies
the caller's Git repository.

`nexus update` downloads the immutable `main-COMMIT` release from the fixed Nexus
GitHub repository. A size-bounded manifest binds every platform asset to its
full commit, semantic package version, byte size, and SHA-256. The updater checks
the staged executable's own `--version` before touching the live installation,
serializes updates with a private installation lock, and atomically replaces an
owned executable in its existing directory. The previous binary remains available
for rollback until setup refresh and the managed service transition succeed.
Setup refresh uses `setup --refresh`, which also works on headless machines without changing the ordinary setup requirement for an installed host. It retains the explicit absolute configuration argument and merges
host settings through the existing setup boundary. Agent applications and their
already running MCP processes are not restarted; the report asks those hosts to
reconnect their companions.

Local daemons expose a same-user control operation only on their private Unix
socket. An exact instance identity binds the resolved configuration, sources,
configuration hash, executable, and working directory. Once a stop is acknowledged, the updater waits for admitted requests and background work to finish; long drains report progress while the old binary remains installed. A managed update drains
the running instance before restarting with that same snapshot. An inactive local
mode remains inactive. A legacy daemon without verifiable control, a daemon
owned by a different executable, or an external remote service is reported as
unmanaged: client and setup assets can update, but Nexus does not claim that the
service changed or guess a process to terminate.

The Compose startup script automatically records management evidence when the
installed CLI supports updates. Source adoption must match the running
service's authoritative Compose configuration hash, with declared environment
values retained from the actual container. Physical mount and network names,
access modes, and aliases must also match. Changed or unprovable settings are
refused before the executable or service is replaced. Retained snapshots bind a
fingerprint of the running configuration; manual runtime changes require fresh,
verified adoption instead of silently applying a stale snapshot. Legacy stacks can be adopted from the exact
configured loopback endpoint, running container Compose labels, and the known
source Compose files. `--compose-file PATH` disambiguates a source file only when
it matches those recorded container labels; a caller's current directory is never
ownership evidence. A retained private resolved Compose snapshot permits future
updates after the original source checkout is gone. It preserves the running
container's environment, health and security settings, volume names, mounts,
networks, and project identity. Named resources are retained as exact external references so an update cannot recreate or reconfigure their backing stores. Compose interpolation is escaped when materializing snapshots so dollar characters in existing secrets remain literal. These files can contain secrets: they are owned
by the current user, live under the per-user `nexus/updates` data directory, use
private directory/file permissions, and are never printed in reports.

Compose updates inspect the actual local Docker server OS/architecture, verify
its matching release archive and image ID, load it, and recreate only the `nexus`
service with `--no-build --no-deps --pull never`. They do not recreate PostgreSQL,
run the importer, remove volumes, or erase persisted data. A failed setup or
service transition restores the previous executable and attempts to restore the
previous managed service image/configuration. Database contents remain in place;
this mechanism does not promise to reverse migrations or external writes.

`.github/workflows/release.yml` publishes only after the `ci` workflow succeeds
for an exact push to `main` in this repository. Four native Linux/macOS x64/arm64
executables and two Linux service-image archives must all be present before a
size/checksum manifest is generated. Publication uses a draft until the complete
asset set is uploaded, and a published commit-tagged release is never overwritten
by a retry. If main has passed CI but its release is still building, update reports
`release_pending` and leaves the installation untouched. The first release is
published by this same gate after this implementation reaches main.

The updater entry point is `src/updater/mod.rs::run`. Release identity, bounded
GitHub transport, atomic executable installation, managed service transitions,
and Compose evidence have separate typed boundaries. Tests use loopback HTTP,
private temporary installations, fake Docker commands, and private local daemon
fixtures; they never update the developer's installed binary or live service.

## Sharing an independent clone

From a new clone, `nexus install --share-project-with /path/to/existing/checkout`
onboards it into the existing checkout's resolved remote project namespace. Both
checkouts must resolve to the same remote service. The installer locks and
atomically updates only `client.project_id` in the target Git-common-directory
configuration, preserving other settings and never copying endpoint or credential
configuration. Local transport and different service endpoints are refused.
Ordinary `install` remains idempotent and preserves an existing binding.
Environment overrides retain their normal precedence; inspect `nexus config show`
in both environments. Old records in the previous namespace are retained.

Machine setup removes obsolete Nexus MCP and native work-event handlers individually,
including within mixed groups, while preserving unrelated handlers. Repeated setup
leaves one generated handler per supported boundary instead of multiplying timing
events.
