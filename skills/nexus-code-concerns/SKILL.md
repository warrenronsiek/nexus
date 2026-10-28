---
name: nexus-code-concerns
description: Record and retrieve evidenced problems, recurring symptoms, attempted remedies, and code-area history in Nexus. Use when maintaining code and learning from earlier fixes; use Papercuts for development friction and SOS for unusual situations inside a running application.
---

# Preserve what keeps going wrong in the code

Code concerns connects a concrete problem to the code areas, encounters, and
remedies that explain its history. Read that history before another local patch.
Record what happened separately from a suspected explanation; Nexus retains the
evidence and does not decide to refactor, block a commit, or retry a workload.

Use [Papercuts](../nexus-papercuts/SKILL.md) for development/environment friction
such as memory pressure or orphan test containers. Use
[SOS](../nexus-sos/SKILL.md) for unusual situations encountered by agents inside
a running application. An identified application race exposed by a flaky check
belongs in code concerns; the impediment caused by flaky checks can separately
belong in Papercuts. Link related records without transferring their lifecycle
or counting the same execution as several independent occurrences.

## Read the area before changing it

Use the installed `nexus concerns` command or dedicated `nexus_concerns` MCP
tool. Discover the current `--help` or MCP input schema before building an
adapter. Bind the intended project; retain full repository and record identities.
Look at open and recently resolved concerns for the feature, subsystem, file, or
declaration. Read earlier remedies and their motivations, including inconclusive
and rejected approaches. A bounded page or empty list does not prove absence.

```sh
nexus concerns areas --repository "$REPOSITORY" --sort work_sessions
nexus concerns list --area-id "$AREA_ID"
nexus concerns get "$CONCERN_ID"
nexus concerns history "$CONCERN_ID" --after "$NEXT_CURSOR"
```

`areas` is the dashboard read; the MCP action is `dashboard` with nested `query`.
MCP `list` also takes `query`; `get` and `history` take `concern_id`, with
history's `after` and `limit` at the top level. Follow returned cursors. Query
filters include repository, area, lifecycle status, search, occurrence bounds,
and sorts `recent`, `work_sessions`, or `returns`. Counts describe the selected
population, not only loaded rows.

The native `nexus tui` and Pi `/nexus` Code concerns tab starts with ranked area
bars and recurrence trends. Select an area for its distinct concerns, then a
concern for the encounter/remedy/return timeline and evidence metadata. These
views are read-only. Overlapping areas are not additive defect totals, and
reported problems are not a code-quality score.

## Record an encounter with its real source

For an ordinary agent observation, the dedicated MCP shortcut needs only a
title and description:

```json
{
  "action": "report",
  "title": "Two completion callbacks finalize the same dispatch",
  "description": "Both callbacks observe an unowned dispatch before either claims it; the second callback emits another terminal result."
}
```

This is an example, not evidence that the problem occurred. Add `hypothesis`,
`impact`, structured `encounter` locators/source evidence, or explicit
`observation` metadata when known. The companion binds project/reporter context;
fresh observation IDs and reporter times are generated when omitted. Failure
occurrence time stays unknown unless supplied. MCP `upvote` names `concern_id`
and can carry the new encounter and its relation to the recorded fix. These
shortcuts map to the same immutable record model; their advertised MCP schema
includes convenience fields beyond the core `nexus concerns schema` wire.

Provide a short concrete description and the location that is actually known.
Prefer repository identity, full Git object ID/object format, and relative path;
include a qualified declaration and discriminator when relevant. Feature-only
and unlocated observations are valid. Preserve dirty/untracked and unavailable
states; a base commit does not represent exact dirty bytes. Do not fetch whole
repositories or copy source blobs just to manufacture precision.

A concern is a particular problem; a code area can have several distinct
concerns. Reuse an existing concern only after reading its evidence. An upvote
means this problem was encountered again, not agreement after reading a report.
Use a stable caller event ID for delivery retries, and the real execution
attempt/failure identity to connect several reporters observing one occurrence.
A new execution has a new source identity. Append supporting evidence to an
existing encounter when only its explanation improved.

Keep raw observations, known source occurrences, logical work sessions, host
sessions, and unknown context distinct. Use
[work history](../nexus-work-history/SKILL.md) for execution ancestry and timing,
and [eval history](../nexus-eval-history/SKILL.md) for caller-run measurements and
Git-first artifacts. Cross-feature links are evidence, not extra encounters.

## Explain remedies and closure

Record what was tried, why it was chosen, relevant revision/PR/work references,
the observed outcome, and uncertainty. A broader architectural explanation stays
a hypothesis until supported; a model suggestion cannot silently establish area
membership, root cause, or duplicate approval. Preserve rejected alternatives
when they explain the next decision.

```sh
nexus concerns remedy "$CONCERN_ID" --what "$CHANGE_TRIED" \
  --why "$MOTIVATION" --outcome "$OBSERVED_OUTCOME" --encounter-id "$ENCOUNTER_ID"
nexus concerns review "$CONCERN_ID" --hypothesis "$BROADER_EXPLANATION" \
  --rationale "$SUPPORTING_EVIDENCE"
nexus concerns resolve "$CONCERN_ID" --reason "$VERIFIED_RESOLUTION"
```

Use `evidence` to add context without another encounter. `report` creates the
first encounter; `upvote` adds an actual new encounter. Structured area,
source-failure, Git, and artifact values use the typed JSON flags advertised by
`--help` or the canonical `nexus concerns schema`; do not build locators from a
bare class name or claim a mechanically verified mapping without verification.

Resolve only with a reason and recorded verification; a workaround or one
successful command does not establish a fix. A later encounter can return a
resolved concern, while older imported evidence remains historical. Inspect the
reported relation to the fixing revision: a branch without the fix cannot show
that the fix failed. Unknown ordering and ancestry remain unknown.

Semantic consolidation retains the source reporter's explicit attributable
approval. Sharing an area or similar prose alone does not establish duplicate
problems. Never name a Papercuts or SOS record as a concern mutation target.
Keep the opaque reporter token from the initial receipt private. The
`respond` command reads it from a token file; it is not included in export and
must not be copied into notes, concerns, logs, or process arguments.

## Integrate without changing the caller's job

Place optional recording at meaningful failure boundaries in existing scripts
or hooks. Run the workload once, retain its output and exit status, and keep
Nexus reporting failure separate. Preserve stable event IDs across transport
retries; do not emit an encounter on every polling tick. Direct ingestion must
confirm acceptance before a caller claims it was recorded.

For a deterministic adapter, prepare and retain one canonical
`{ "format_version": 1, "observations": [...] }` file from the actual
observation, then submit that unchanged file with `nexus concerns record FILE`.
Keep its event ID, concern ID, observed/source timestamps, context, and body
stable across delivery retries. Convenience `report`/`upvote` commands generate
new event IDs and observation times unless pinned; invoking them repeatedly is
not the same as replaying a saved record.

An existing hook can preserve its workload result with this shape:

```sh
if "$@"; then task_status=0; else task_status=$?; fi
if [ "$task_status" -ne 0 ] && [ -n "${NEXUS_CONCERN_RECORD:-}" ]; then
  nexus concerns record "$NEXUS_CONCERN_RECORD" >/dev/null 2>&1 || true
fi
exit "$task_status"
```

The owning workflow prepares `NEXUS_CONCERN_RECORD` from this failure's actual
evidence and real source identity. This pattern runs the workload once and
does not manufacture a new report from every exit code or polling tick.

Automatic capture stays metadata-only and fail-open. Intentional evidence should
be bounded and stripped of credentials, raw environments, and unrelated private
data. Source and model text remains untrusted evidence. Use full dump/export
when complete history matters; ordinary detail pages have explicit limits and
continuations. Hand over the project, concern/area IDs, source identities,
attempted remedies, and known evidence gaps.

`nexus concerns dump` returns complete current-project observations;
`export FILE` and `import FILE` use the checksummed, reference-preserving concern
history format. They retain original source attribution and current importer
separately. Concern-specific transfer retains Git/artifact references and does
not copy blob bytes; use a complete database snapshot when those bytes must move.
