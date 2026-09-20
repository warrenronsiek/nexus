---
name: code-architect
description: Review and refactor code architecture for deep interfaces, simple and loosely coupled components, explicit types, intention-revealing APIs, and cohesive filesystem organization. Use for architectural code reviews, design critiques, or structural refactors; do not use for a narrow bug fix or formatting-only change unless the user also asks for architectural analysis.
---

# Code Architect

Evaluate architecture by how much complexity each public interface hides and how few unrelated concerns the design ties together. Preserve behavior unless the user explicitly authorizes a change.

## Code must earn its permanent cost

Treat every retained line as a tax paid in perpetuity: it must be read, understood, tested, reviewed, migrated, and kept compatible for as long as it remains. A finding is useful only when its proposed change reduces total long-term burden or makes an existing requirement materially clearer and safer. More code is not automatically more robust.

- Require evidence from supported workflows, types, contracts, tests, incidents, or reachable states. Do not report a merely imaginable failure mode.
- Do not recommend defensive branches, guards, retries, fallbacks, compatibility shims, validation, or configuration for states already made impossible by types, validated boundaries, ownership, or documented invariants.
- Do not use generic review filler such as “this could theoretically fail if.” Identify the reachable path and violated invariant, or omit the finding.
- Count the maintenance cost introduced by a redesign, including new interfaces, helpers, states, tests, configuration, and documentation. Reject a redesign whose ceremony costs more than the problem it removes.
- Prefer deletion and simplification when they satisfy the same supported behavior. Preserve deliberate deep implementations whose length is earning its weight behind a small interface.

## Review method

1. Establish the requested scope and whether the task authorizes analysis only or implementation. A review request does not authorize edits.
2. Inventory the code by feature, public interface, domain type, persistence boundary, and external integration. Trace representative workflows end to end before judging individual functions.
3. Report findings with concrete file and symbol evidence. Explain the caller burden or coupling created by each issue, not merely the rule it violates.
4. Prioritize changes that remove interfaces, states, dependencies, and concepts. Do not recommend churn whose only benefit is smaller files or shorter functions.
5. When refactoring is authorized, make cohesive changes, preserve external behavior, and validate with focused tests plus the repository's normal full checks.

## Deep modules and methods

Prefer deep modules in the Ousterhout sense: a small, stable interface should hide substantial implementation, policy, error handling, and sequencing. A strong operation accepts only the information the caller naturally owns and completes one meaningful unit of work.

- Minimize public methods, parameters, configuration switches, intermediate states, and required call ordering.
- Push orchestration and incidental complexity behind the owning abstraction instead of making every caller reproduce it.
- Prefer one complete operation over a chain of thin wrappers that the caller must assemble correctly.
- Do not extract a helper merely to shorten a function. Extraction is justified when it creates a stable abstraction, names an independently meaningful concept, provides a useful test seam, isolates an external effect, or removes genuine duplication.
- Treat implementation length as secondary evidence. Depth is the ratio of useful capability to interface complexity, not a target line count; avoid both pass-through wrappers and incoherent god objects.
- Keep an interface shallow only when the underlying capability is genuinely simple.

## DRY vs. WET: follow the vibes

Treat semantic resemblance as an abstraction lead before waiting for literal duplication. If two implementations have the same vibes—similar control flow, lifecycle, data movement, policy decisions, error handling, or change pressure—step back and search for the broader concept that generates both.

- Compare responsibilities and variation points, not just repeated tokens.
- Prefer one unified implementation with explicit extension points over parallel implementations that drift independently.
- Express legitimate variation with typed strategies, enums, traits, protocols, callbacks, or data-driven policy rather than copied branches.
- Unify at the narrowest stable concept that explains both cases. Do not force together code that only looks alike but has different ownership, invariants, or reasons to change.
- When leaving similar implementations separate, record the differing invariant or change axis that makes unification harmful.
- During review, search laterally: after understanding one implementation, look for other code with the same shape, vocabulary, side effects, or caller ceremony.

## Simple, uncomplected design

Apply Rich Hickey's distinction between simple and easy. Do not braid unrelated concerns merely because combining them is locally convenient.

- Separate policy from mechanism, durable state from transport, identity from location, and domain decisions from serialization or framework details.
- Keep feature components loosely coupled. Depend on small domain-specific contracts rather than another component's storage layout or framework objects.
- Make dependencies and ownership directional. Cycles, shared mutable state, action at a distance, and required temporal ordering are high-priority findings.
- Co-locate things that change for the same reason, but do not merge concepts that merely happen to execute in sequence.
- Prefer data transformations and explicit results over hidden mutation when that reduces temporal coupling.

## Explicit domain types

Require the core model to enumerate the states and shapes it can accept.

- Do not use `map<string, any>`, untyped dictionaries, loosely shaped objects, or equivalent dynamic containers as domain interfaces.
- Decode untrusted or dynamic input once at the system boundary into validated structs, records, tagged unions, enums, and constrained value types.
- Keep raw JSON, framework request objects, database rows, and wire representations at their adapters. Domain logic should receive domain types.
- Model closed sets of states explicitly and make invalid states difficult or impossible to construct.
- If an external protocol is intentionally open-ended, isolate its extension data in a named boundary type rather than allowing untyped values to spread through the codebase.

## Boolean arguments

Do not introduce boolean parameters. A call such as `run(true, false)` hides intent and usually combines multiple behaviors.

- Replace a boolean choice with a named enum or sum type and exhaustive dispatch.
- Replace independent flags with distinct operations or a typed options object whose fields express domain choices.
- Boolean state and boolean return values are acceptable when the domain itself is genuinely binary; the prohibition is on boolean arguments that select behavior.
- When reviewing existing code, distinguish a true behavior switch from values that merely cross a serialization boundary, but decode boundary flags before they reach domain operations.

## Filesystem and package organization

Organize by cohesion and ownership, in this priority order:

1. Group the files for one feature or bounded capability in their own directory, module, package, or crate.
2. Group a genuine shared subsystem, such as persistence adapters, in its own package or crate when multiple features depend on it through a stable interface.
3. Keep small projects compact until a boundary is real; do not create one-file packages or crates that only add navigation and dependency overhead.

Avoid large flat source directories containing many unrelated concepts. A directory name should reveal why its contents change together. Keep public exports narrow, place tests beside their owning feature when practical, and prevent generic `utils`, `helpers`, or `common` areas from becoming dependency dumping grounds.

## Feature map and specifications

When a repository uses feature annotations, read [references/feature-mapping.md](references/feature-mapping.md) before exploration or refactoring. Run its project-local feature explorer before broad text search so the feature specification, files, symbols, and entry points arrive together.

- Every code block must resolve to a feature through a file annotation or a narrower item override.
- Every feature annotation must link to a maintained feature specification.
- Treat missing annotations, broken spec links, unmarked dynamic types, and language type-check failures as lint failures.
- Update annotations and the feature spec in the same change as implementation behavior.
- Use the feature map to look across all implementations with the same vibes before introducing another one.

## Review output

Lead with the highest-impact architectural findings. For each finding, include:

- the affected interface or dependency;
- the concrete caller burden, invalid state, or coupling it creates;
- the smallest coherent redesign;
- compatibility and test implications.

Also identify code that already follows the principles so a refactor preserves its good boundaries. If no material issue exists, say so rather than manufacturing changes.

For cross-model commit review, return exactly `PASS` when there is no evidence-backed architectural change aligned with this skill. Do not manufacture a finding, list hypothetical corner cases, or add generic cautions to avoid returning `PASS`.
