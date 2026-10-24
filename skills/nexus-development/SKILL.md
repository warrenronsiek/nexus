---
name: nexus-development
description: Develop Nexus through feature discovery, typed persistence, structural analysis, and staged commit review. Use when changing Nexus source or schemas, diagnosing its repository checks, or integrating its advisory commit reviews into an existing Git workflow.
---

# Extend Nexus without changing its role

Nexus connects durable observations to the work that produced them. Its source
tooling keeps those features understandable as they cross domain, persistence,
runtime, and terminal interfaces. Preserve the owning feature's invariants while
changing that flow; a new transport or screen should not invent different domain
rules. Lifecycle capture remains advisory and fails open.

For choosing which Nexus capability belongs in an existing workflow, start with
the sibling [Nexus guide](../nexus/SKILL.md). For machine installation, project
binding, configuration, or service deployment, use
[nexus-setup](../nexus-setup/SKILL.md).

## Resolve the feature before editing

In a **Nexus source checkout**, run:

```sh
python3 scripts/feature_map.py --root . --list
python3 scripts/feature_map.py --root . --feature coordination
```

Replace `coordination` with the relevant feature. Read the returned specification
and entry-point files, then follow its data flow into the affected layers. The
feature list is authoritative; it is not limited to the example above. Update
behavior, its specification, and the relevant bundled skill together. Add a new
feature to the Nexus guide's routing table so callers can discover it.

Every code file declares leading `@feature` and matching `@spec` annotations.
Symbols inherit them. Mark a small number of useful `@entrypoint` starting points;
give intentional open-ended decoding an explanatory `@boundary`. Prefer a shared
typed implementation when lifecycle, policy, errors, and reasons to change agree.
Similar syntax alone does not establish that agreement.

Read [source structure and complexity](references/source-structure.md) for
annotations, feature specifications, structural metrics, or repeated code shapes.
Read [persistence and validation](references/persistence-validation.md) for schema
changes, durable state, canonical checks, or cross-model commit review.

These source scripts belong to Nexus. Installing this skill does not install a
feature-map script, quality gate, or tracked hook into another project's tree.
The installed `nexus review-commit` command can review that project's staged diff;
the reference explains how to connect it to a caller-owned hook.

## Connect the change to existing work

Trace an observable operation from its host adapter through typed domain/service
logic to storage and back to retrieval. Preserve project scoping, identity,
replay, privacy, pagination, timing, and history semantics where the owning
feature defines them. Review the native/Pi terminal surface when new durable data
needs to be visible. Do not require callers to access Diesel tables or reconstruct
private rows themselves.

For integration contracts, consult the relevant installed domain skill before
adding a new API. Use [work history](../nexus-work-history/SKILL.md) to connect
tests, reviews, agents, and delivery evidence; use
[eval history](../nexus-eval-history/SKILL.md) for what was evaluated, its
motivation, metrics, and artifacts. Those recording features retain caller-owned
execution and success criteria.

Validate the affected behavior and complete Nexus's canonical `./scripts/check.sh`
gate. Treat structural metrics as evidence for design decisions and model review
as advisory findings. Report actual validation results, unresolved findings, and
provider failures separately; none is a substitute for the others.
