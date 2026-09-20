# Feature mapping convention

Use this reference when a repository has `@feature` annotations or when establishing feature-aware exploration and linting.

## Repository contract

Keep the repository-specific commands and feature catalog in its root `AGENTS.md`. The conventional explorer path is `scripts/feature_map.py` and supports:

```sh
python3 scripts/feature_map.py --root . --feature <tag>
python3 scripts/feature_map.py --root . --lint
```

The first command prints the feature's specification, files, declared entry points, and discovered symbols. The second verifies the annotation and specification graph. Pair it with the language's strict type checker in the normal lint command; the explorer supplements rather than replaces the compiler.

## Source annotations

Put annotations in the leading comment block of every code file:

```text
@feature coordination
@spec docs/features/coordination.md
@entrypoint NexusService::handle
@boundary dynamic-json
```

Use the language's ordinary comment marker. `@feature` and `@spec` are required. `@entrypoint` is repeatable and identifies the small number of useful starting symbols. `@boundary` is optional and must name an intentional dynamic boundary such as `dynamic-json`, `dynamic-config`, `generated-code`, or `untyped-foreign-api`.

A file may declare multiple features and specs when it genuinely joins them. All discovered functions, methods, classes, structs, enums, traits, records, and other named blocks inherit the file's feature set. If a repository needs finer attribution in a mixed file, its explorer may support item-level overrides, but do not repeat annotations mechanically when file ownership is already precise.

## Feature specifications

Each referenced Markdown file begins with frontmatter containing the matching tag:

```yaml
---
feature: coordination
---
```

Write the body for a smart but tired first-time reader, in this order:

1. a plain-language statement of what the feature accomplishes;
2. the motivation and user or system problem it solves;
3. a flowchart showing inputs, decisions, state changes, outputs, and failure paths;
4. an explanation of every node and edge in that flowchart;
5. implementation details: entry points, domain types, persistence, integrations, invariants, and tests.

Prefer one readable end-to-end diagram over several decorative diagrams. Keep implementation detail subordinate to the feature's purpose and flow.

## Lint expectations

The repository explorer should fail when:

- a scanned code file lacks `@feature` or `@spec`;
- a referenced spec is missing or its `feature` frontmatter does not match any declared feature;
- a declared feature has no entry point anywhere in the repository;
- an open-ended dynamic type appears without a named boundary annotation;
- a supported dynamic-language function lacks parameter or return annotations.

The repository's normal type checker remains authoritative for the language. For example, run Rust compiler and Clippy checks, `mypy --strict`, or `tsc --noEmit` alongside feature-map lint.

## Change discipline

Before editing, query the relevant feature tag and read its complete spec plus the returned entry-point files. After editing, rerun feature-map lint and the language checks. If new code has the same vibes as another feature path, evaluate unification before adding a parallel implementation.
