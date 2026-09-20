---
name: code-deletion
description: Review code changes and repositories for obsolete implementations, unnecessary compatibility layers, unused code, deprecated behavior, stale configuration, and artifacts that should be removed. Use after refactors, replacements, migrations, feature removals, or before commits; do not use to delete behavior the user still requires or historical database migrations needed to upgrade existing installations.
---

# Code Deletion Review

Review for code that no longer earns its maintenance cost. The default deliverable is evidence-backed deletion findings; do not edit files unless the user has authorized implementation.

## Net-negative gate

Application of this skill must reduce the total maintained codebase. A proposed cleanup qualifies only when its complete implementation deletes more lines than it adds across production code, tests, configuration, documentation, fixtures, and adapters.

- State the estimated lines removed, lines added, and net LOC delta for every proposed deletion set.
- The net delta must be strictly negative. Moving, renaming, wrapping, replacing, or regenerating the same code elsewhere does not count as deletion.
- Do not propose added compatibility machinery, generalized abstractions, defensive checks, or new tests whose combined cost makes the deletion set net neutral or net positive.
- When cleanup is implemented, verify the actual aggregate diff is net negative. If required behavior cannot be preserved with a negative delta, do not apply this skill to that candidate.
- If no safe, cohesive, strictly net-negative deletion exists, return exactly `PASS`.

## Review method

1. Read the change and identify every behavior it replaces, renames, migrates, or removes. Search the whole owning feature, not only changed files.
2. Trace both the new path and the suspected old path through entry points, callers, configuration, persistence, tests, documentation, and generated integration surfaces.
3. Build a deletion set rather than naming one line. Include obsolete implementations plus their exports, adapters, flags, data types, tests, fixtures, documentation, and dependencies when they exist only for that path.
4. Distinguish required transition machinery from speculative compatibility. A compatibility mapping, dual-write, fallback, legacy alias, version switch, or deprecated wrapper needs a named consumer, removal condition, and intended lifetime. Without those, flag it for deletion.
5. Check generic residue: unreachable branches, unused private code, unreferenced dependencies, stale feature flags, superseded schemas, redundant tests, commented-out implementations, TODO removal markers, and deprecated APIs with no supported consumers.
6. Verify safety before recommending deletion. Internal references are strong evidence for private code; lack of internal references alone does not prove that a public API, plugin hook, reflection target, serialized field, or externally loaded entry point is unused.
7. Report concrete findings ordered by confidence and impact. If cleanup is authorized, delete cohesive sets and run the repository's complete validation.

## Refactor and migration rules

- Prefer one current implementation. Do not keep the old and new paths in parallel merely because removing the old path feels risky.
- Do not create compatibility aliases or translation maps unless the task identifies an old consumer that must continue to work.
- When compatibility is required, require an explicit boundary, owner, removal trigger, and test. Temporary compatibility without an exit plan is permanent architecture.
- Remove old configuration properties, environment overrides, commands, documentation, and tests in the same change that removes their behavior.
- Treat deprecation as a transition contract, not a substitute for deletion. Flag expired or unowned deprecations.
- Preserve data needed for upgrades and rollback. Applied database migration scripts are immutable history and are not dead code; add a subsequent migration instead of rewriting or deleting an old one.
- Preserve intentionally retained protocol fields and public interfaces when compatibility is an explicit product requirement, but make that requirement visible in types, documentation, and tests.

## Cross-model commit review

When invoked as one pass in a parallel commit review, stay focused on deletion. Let the code-architecture pass judge module depth and coupling. Inspect the staged change plus enough repository context to answer:

- Did a replacement leave the replaced implementation reachable?
- Did the change add a compatibility bridge without an identified consumer?
- Are old names, flags, types, storage fields, tests, docs, or dependencies still present?
- Did an apparent feature removal leave alternate entry points or generated registration behind?
- Can several findings be resolved by deleting one obsolete concept and all of its satellites?

Return exactly `PASS` when there is no actionable, strictly net-negative deletion. Otherwise return `CHANGES_REQUESTED` with file and symbol evidence, why the code is obsolete, the complete deletion set, estimated removed and added LOC, the negative net delta, and any verification needed before removal. Never invent cleanup solely to avoid `PASS`.

## Finding format

For each finding provide:

- confidence: high, medium, or low;
- affected files and symbols;
- evidence that the code is obsolete or the compatibility layer is unjustified;
- the cohesive deletion set;
- compatibility or data-migration risk;
- estimated removed LOC, added LOC, and negative net delta;
- focused verification after cleanup.

Separate confirmed dead code from investigation leads. A deletion review succeeds by removing obsolete concepts safely, not by maximizing deleted lines.
