---
name: code-complexity-analysis
description: Analyze maintainability and repeated implementation shapes with big-code-analysis at repository, feature, file, or symbol scope. Use for complexity hotspots, abstraction decisions, and before/after refactor comparisons; inspect source before treating a score as a design judgment.
---

# Code Complexity Analysis

Use measurements to locate reader and caller burden. Keep behavior and scope
fixed when comparing designs. Prefer deep, cohesive operations and explicit
domain types; use [functional-rust](../functional-rust/SKILL.md) for Rust design
choices and [code-architect](../code-architect/SKILL.md) for architectural review.

## Measure the owning scope

In a Nexus source checkout, resolve the feature before searching broadly:

```sh
python3 scripts/feature_map.py --root . --list
python3 scripts/feature_map.py --root . --feature complexity-analysis
```

Use the project wrapper for feature, file, and symbol selection:

```sh
python3 scripts/complexity_analysis.py --feature coordination --format json
python3 scripts/complexity_analysis.py --path src/persistence/store.rs
python3 scripts/complexity_analysis.py --path <file> --symbol <name>
python3 scripts/complexity_analysis.py --check
```

Without a scope it analyzes annotated authored source. These commands belong to
Nexus; installing this skill does not add them to other repositories. Elsewhere,
prefer an existing project adapter or run `bca metrics --format json <paths>`.
Resolve feature tags to paths first; BCA does not own a repository's feature map.
An unavailable analyzer is a tooling failure, not a clean report.

## Interpret the objective

`complexity.toml` owns the configurable ranking weights. Read the report's actual
formula and contribution breakdown rather than assuming default coefficients.
The score charges named callable declarations and arguments more than logical
implementation lines. Rust mutation syntax, recognizable state containers, and
dynamic type syntax add separate costs. Immutable bindings and enumerated domain
types retain raw evidence without the expensive callable-declaration surcharge.
Closures used by ordinary iterator composition do not create a new named API.

Use the overlap-free scope total for before/after comparisons. File and container
scores include their descendants: adding every hotspot score counts the same
code repeatedly. Inspect raw BCA metrics and individual weighted contributions to
explain the movement. Compare the same files or feature and record any change to
the scoring configuration.

Static syntax cannot prove purity, resolve every alias, discover all hidden
dependencies, or measure semantic cohesion. Rust-only proxies are not evidence
that another language has no state or dynamic typing. Check adapters and type
definitions before interpreting a state or dynamic-type finding. Deliberate
external JSON boundaries and necessary resource mutation can be justified.

## Review the code that moved

- Preserve deep operations that hide coherent work behind a small interface.
  Do not extract a helper merely to lower one function's branch or line count.
- Do not lower arity by moving inputs into captured mutable state, positional
  tuples, a shapeless options bag, or an object with a required call sequence.
- Keep explicit structs, enums, typed errors, and exhaustive matches when they
  remove ambiguity. Deleting types, compressing lines, expanding macros, or
  cloning data to erase `mut` does not establish an improvement.
- Compare repeated shapes by policy, ownership, lifecycle, failure semantics,
  and reasons to change. Share a typed concept when those agree; retain separate
  implementations when their invariants differ. Token overlap is only a lead.
- Keep measured ranking separate from enforcement. Nexus `--check` runs the
  committed BCA thresholds/baseline and strict near-copy gate; the preference
  score ranks review targets. Do not raise thresholds or refresh a baseline
  merely to hide a regression. Exit 1 indicates a tool/input error; exit 2
  indicates a gate violation in the wrapper.

Report the behavioral scope, ranked targets, raw metrics and contributions,
overlap-free total before/after, and the concrete design improvement. Include
healthy deep implementations and false positives when relevant. A lower number
alone is not a finding or proof of better code.
