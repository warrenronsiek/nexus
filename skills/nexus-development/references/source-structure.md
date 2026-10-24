# Feature-shaped source work

All commands and source paths here are relative to a Nexus checkout. They are not
requirements on repositories that merely use Nexus.

## Architecture tooling

`scripts/feature_map.py` reads source annotations, resolves feature specs, and
prints owned files, symbols, and entry points. Use `--list`, then `--feature NAME`
before editing. For a cross-cutting operation, inspect each affected feature;
directory ownership alone does not explain its invariants.

Put the annotations in the file's leading comment block, using that language's
comment syntax. For example, a Rust file intentionally decoding external JSON
might begin:

```rust
// @feature coordination
// @spec docs/features/coordination.md
// @entrypoint observe_external_event
// @boundary external-host-json: decode host input into validated domain records
```

The names are examples; reference an actual spec and useful entry point for the
implementation. Multiple feature/spec pairs are allowed when ownership is shared.
Annotations apply to every discovered symbol in a file. Keep open-ended JSON,
TOML, or string-keyed dynamic data at explained boundaries, then convert it to
explicit domain types. Python tooling also requires annotated function signatures.

A feature spec has `feature: NAME` frontmatter and explains plain-language
purpose, motivation, one end-to-end flowchart, every node and edge, and
implementation entry points and invariants. Change that description with the
behavior. Feature-map lint detects missing links and boundaries; it cannot prove
the prose still describes the implementation.

```sh
python3 scripts/feature_map.py --root . --lint
```

Start implementation investigation at `scripts/feature_map.py`,
`docs/features/architecture-tooling.md`, and the relevant feature's returned files.

## Complexity analysis

`scripts/complexity_analysis.py` selects annotated source through the same map and
invokes the installed `bca` analyzer. CI pins `big-code-analysis-cli==2.2.0` in a
managed Python runtime. Use a scope that answers the design question:

```sh
python3 scripts/complexity_analysis.py --feature coordination
python3 scripts/complexity_analysis.py --path src/persistence/store.rs
python3 scripts/complexity_analysis.py --path src/agents/reviewer.rs --symbol staged_diff --format json
python3 scripts/complexity_analysis.py --check
```

Normal mode reports advisory repetition candidates; `--check` runs committed BCA
thresholds/baseline plus the strict near-copy gate. `bca.toml` and
`.bca-baseline.toml` own metric enforcement; `complexity.toml` owns ranking weights
and repetition thresholds. An absent analyzer is a tooling failure, not a clean
report. Use the current script's `--help` for supported scope options.

The declaration-weighted composite ranks attention and retains raw metrics. It
does not define quality by itself. A long cohesive operation may hide complexity
better than several shallow wrappers. Before merging repeated shapes, compare
their policy, lifecycle, failure semantics, ownership, and reasons to change.
Share the broader typed concept when those match; retain separate implementations
when their invariants differ and explain why in review.

Address a gate failure in its actual implementation or justify a deliberate policy
change on its merits. Raising thresholds or refreshing the baseline merely to
hide a new regression does not establish an improvement.

When changing the tooling itself, read `docs/features/complexity-analysis.md`,
`scripts/complexity_analysis.py`, and its focused tests. Keep exploration and
enforcement on the same ownership, tokenization, and metric contracts.
