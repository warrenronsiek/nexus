# Durable state and review

Source paths and `./scripts/*` commands in this reference belong to a Nexus
checkout. The installed `nexus review-commit` command is also usable in a caller's
Git repository.

## Persistence

Read the affected domain spec and `docs/features/persistence.md` before changing
storage. `src/persistence/store.rs` provides `Store::open` for SQLite and
`Store::connect` for PostgreSQL; `backend.rs` selects exactly one compiled backend.
Feature-scoped modules keep their Diesel rows private and return validated domain
records. Add behavior through that interface rather than exposing table layouts
to callers or UI code.

- Use Diesel's typed query DSL for application reads and mutations. SQL schema
  definitions belong in migrations, not a parallel runtime query path.
- Add a later `V<version>_<description>.sql` migration to both `migrations/` and
  `postgres_migrations/`, with equivalent backend-specific semantics. Released
  migrations are immutable; preserve historical files and checksums. Update the
  typed Diesel schema and affected row conversions in the same change.
- Group authoritative writes and their associated audit/projection changes in
  the appropriate existing transaction helper. Preserve commit-failure rollback
  and bounded contention behavior; a failed operation must not poison the next
  request or leak uncommitted state.
- Preserve project-local identity and provenance. Bound ordinary queries in the
  database before materializing rows, and expose truncation/cursors. An explicitly
  complete history/export contract is different from a bounded summary.
- Distinguish authoritative history from rebuildable indexes and summaries using
  the owning feature's contract. Do not overwrite an immutable observation to
  repair a projection or treat a disposable summary as the only evidence.

New durable tables also affect `src/persistence/transfer/stream.rs` and its typed
rows. Keep complete snapshots, dependency ordering, validation, identity, and
format compatibility intact. Import is an atomic transfer into an empty target,
not a general database merge. Git artifact references preserve coordinates, not
the referenced repository's bytes. Use isolated test databases for migration,
replay, rollback, export/import, and backend-equivalence checks.

The default build is SQLite. PostgreSQL uses
`--no-default-features --features postgres`; enabling both backends is invalid.
The service currently supports one authoritative `nexus-server` process per
database. Changes to concurrency, readiness, or deployment must address that
lifecycle explicitly rather than assuming multiple replicas are supported.

## Canonical validation in Nexus

```sh
./scripts/install-hooks.sh
./scripts/check.sh
```

Hook installation configures this checkout's `core.hooksPath=.githooks`. The
canonical check composes feature annotations, BCA/repetition policy, Python
tooling tests, locked Pi validation, Docker healthcheck regression, Rust format,
Clippy for both backends, and the default Rust suite. Use focused checks while
developing, then complete this shared gate. Honor its test isolation and deadline
policy rather than bypassing assertions or introducing live provider credentials
into fixtures. Rust code must support the version declared in `Cargo.toml`.

CI invokes the same gate on Linux and macOS. Separate jobs exercise PostgreSQL
integration and the production Compose service. A passing local default suite
does not establish that those environment-dependent jobs passed; report their
actual results when relevant to delivery.

Existing check, hook, and review boundaries already record work history. Reuse
those intervals and parent context rather than wrapping them again solely to
duplicate telemetry. Consult the work-history integration skill when adding a
new caller-owned boundary.

## Cross-model staged commit review

Nexus runs two read-only perspectives concurrently: `code-architect` and
`code-deletion`. The configured review provider differs from the implementation
provider. Reviewers receive the staged diff, may inspect repository context, and
return findings or `PASS`; missing skills, provider failures, and timeouts remain
visible advisory results.

In an existing repository with Nexus configured, inspect the staged change and
request review with:

```sh
git diff --cached
nexus review-commit
```

In the Nexus source checkout, the equivalent invocation is:

```sh
cargo run --bin nexus -- review-commit
```

`nexus setup` installs the review skills. Review configuration uses `review.enabled`,
`review.implementation_provider`, `review.provider`, provider command/model
settings, `review.timeout_seconds`, and optional `review.skill_root`. Inspect
resolved configuration with `nexus config show` and validate it with
`nexus config check`; use the setup skill to change configuration deliberately.
The default skill lookup is `$CODEX_HOME/skills` or `~/.codex/skills`.

To integrate with another repository's existing pre-commit workflow, retain its
deterministic checks, then invoke the installed command from that repository. Keep
review failure advisory, as Nexus's own hook does:

```sh
if ! nexus review-commit; then
  printf '%s\n' 'warning: Nexus advisory commit review was unavailable' >&2
fi
```

Integrate this step into the caller's chosen hook mechanism; installing the skill
or running `nexus install` does not install Nexus's source `.githooks` gate there.
Review consumes configured provider resources. Use it within the authorized
workflow and record whether it ran, was disabled, had no staged changes, or failed.

Assess concrete findings against the implementation and owning feature's
invariants. Provider authentication or input-limit failures are not failed code
tests; a successful model invocation is not proof that deterministic checks
passed. Nexus's tracked pre-commit hook runs its deterministic gate first, then
prints cross-model results without turning them into a commit-denial or agent
permission decision. Keep that distinction when integrating other hooks.
