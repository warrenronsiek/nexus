# Nexus agent guide

Nexus is organized and explored by feature. Before changing code, resolve the relevant feature instead of starting with a repository-wide text search:

```sh
python3 scripts/feature_map.py --root . --list
python3 scripts/feature_map.py --root . --feature coordination
```

Read the returned specification and entry-point files before editing. The current feature catalog is:

- `coordination`: session observation, claims, classification, advisories, and reconciliation.
- `persistence`: Diesel models, migrations, events, and projections.
- `runtime`: CLI, daemon protocol, MCP adapter, and host hook generation.
- `analyst`: explicit Codex or Claude conflict analysis.
- `configuration`: layered configuration, validation, and provenance hashing.
- `architecture-tooling`: feature annotations, exploration, architectural lint, and repository validation.
- `complexity-analysis`: feature-scoped structural metrics, repetition analysis, and regression gates.
- `commit-review`: parallel cross-model architecture and deletion review for staged commits.
- `installation`: one-command machine setup and repository onboarding.
- `observability-ui`: read-only Axum, Elm, and D3 operational visibility.
- `usage-analytics`: observed tool, skill, and script usage capture and seven-day reporting.

Every code file must declare at least one `@feature` and matching `@spec` in its leading comment block. Symbols inherit those file annotations. Declare the small number of useful starting points with `@entrypoint`. Files that intentionally decode open-ended external values must also declare an explanatory `@boundary`.

When two implementations have the same vibes—similar flow, lifecycle, state changes, policy, errors, or caller ceremony—look for the broader shared concept before adding another path. Prefer a unified typed implementation with explicit variation points. Keep implementations separate only when their invariants or reasons to change genuinely differ.

The complete local validation is one command, shared by developer runs, the tracked pre-commit hook, and CI:

```sh
./scripts/check.sh
```

Enable the tracked hooks for this checkout once with:

```sh
./scripts/install-hooks.sh
```

Update the feature annotation and specification in the same change as implementation behavior. Application database access must continue to use Diesel's typed query DSL; SQL strings belong only in migrations. Lifecycle hooks must remain advisory and fail open.
