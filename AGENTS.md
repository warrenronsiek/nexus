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
- `observability-ui`: native and Pi terminal coordination, analytics, and memory visibility over local MCP.
- `usage-analytics`: observed tool, skill, and script usage capture and seven-day reporting.
- `agent-memory`: explicit durable notes, bounded hierarchical context, search, expansion, and Nexus-managed consolidation.
- `papercuts`: timestamped development-friction reports, upvotes, duplicate review, consolidation, resolution, and complete registry export.
- `sos`: immediate agent blockers, repeated-report aggregation, lifecycle tracking, and triage history.
- `evals`: Git-backed evaluation definitions, motivations, caller-owned results, grading history, artifacts, comparisons, and terminal retrieval.
- `work-history`: durable work sessions, nested activity timing, deterministic capture, caller-owned delivery evidence, and terminal timelines.

The bundled [Nexus skill](skills/nexus/SKILL.md) maps every feature to focused
usage and integration guidance. `nexus setup` installs it and the linked skills
for Codex and Claude. Load the relevant guide when integrating Nexus into other
work; use `nexus-development` for changes to Nexus itself.

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

Update the feature annotation, specification, and relevant bundled skill in the same change as implementation behavior. Add new features to the Nexus skill's routing table. Application database access must continue to use Diesel's typed query DSL; SQL strings belong only in migrations. Lifecycle hooks must remain advisory and fail open.
