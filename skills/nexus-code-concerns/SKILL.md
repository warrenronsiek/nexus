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
filters include repository, area, `open` or `resolved` status, search, and sorts
`recent`, `work_sessions`, or `returns`. Use `--occurred-after` and
`--occurred-before` for UTC occurrence bounds. Nexus uses source failure time
when supplied and observation time otherwise, and exposes unknown source-time
counts separately. Counts include the full selected population. Ordinary lists
default to 24 results and cap at 64. Detail starts with eight recent history
entries and reports when more history is available.

The authenticated web interface's Code concerns view starts with the Code map.
Open Problem areas for ranked area bars and recurrence trends. Select an area
for its distinct concerns, then a
concern for the encounter/remedy/return timeline and evidence metadata. The
area rankings and lists default to the last seven days. Use the range control
for 24 hours, 14 days, or 30 days. An exact concern and its history can still
be read outside the selected list window. These views are read-only. One
encounter can contribute to several areas, so adding their counts can count it
more than once.

## Group observations in the code map

Open Code concerns → Code map or run `nexus concerns map`. Select reported
encounters, open concerns, or a published project measure. Group by recorded
file, directory, feature, subsystem, or caller-defined dimensions. Include
scalar labels such as `"dimensions": {"team":"storage"}` inside the encounter.
Keep using structured affected areas for the built-in location dimensions.
Shared and Unattributed buckets preserve one placement per point.
Reported encounters count each report; open concerns count a canonical concern
once and combine its recorded labels. Repeated identical placements stay one
label, differing labels become Shared, and missing labels remain Unattributed.
The same relative file path in different repositories retains separate identity.

Publish an external numeric snapshot with `nexus concerns publish-measure FILE`
or the `nexus_concerns` action `publish_measure`. Supply an immutable ID,
producer, measure key, title, unit, reducer (`sum`, `max`, or `mean`), revision,
observation time and samples. Samples identify a subject, numeric value and
optional string dimensions and evidence reference. Each subject can occur once
per snapshot. Send the same ID and payload on retry. Read the current schema
with `nexus concerns schema`.

`nexus concerns map QUERY.json` takes the same query object as MCP action `map`
under `query`. For a published measure, select its producer and key and optionally
pin the snapshot used for the read.

```json
{
  "measure": {"kind":"published", "producer":"ci", "measure":"branch_load"},
  "group_by": "team",
  "snapshot_id": "branches-4dc2e91"
}
```

Omit `snapshot_id` to select the latest snapshot by observation time. To drill
into a group, pass the returned group key as `bucket` with the same query.
The treemap sizes nonnegative additive measures by value when there is a
positive value. For signed, all-zero, and nonadditive measures, it sizes by
sample count while displaying the exact reduced value. Check `partial` before
treating the displayed groups and evidence as complete. Code Map has its own
bounded data selection and published snapshot identity.

## Record an encounter with its real source

Record the affected code in `encounter.areas` on reports and upvotes. Use the
repository identity, repository-relative file path, and feature or subsystem
keys verified in the repository. Every area has a kind, key, and display name.
Use memberships when the same file belongs to known features or subsystems.

```json
{
  "action": "report",
  "title": "Two callbacks finalize the same dispatch",
  "description": "Both callbacks observed the same unclaimed dispatch and emitted a terminal result.",
  "encounter": {
    "areas": [{
      "relation": "affected",
      "area": {"repository":"example/service","kind":"file","key":"src/dispatch.rs","display_name":"src/dispatch.rs"},
      "memberships": [
        {"repository":"example/service","kind":"feature","key":"dispatch","display_name":"Dispatch"},
        {"repository":"example/service","kind":"subsystem","key":"worker","display_name":"Worker"}
      ]
    }]
  }
}
```

For an actual repeat of that problem, use the recorded concern ID and the areas
known for the new encounter:

```json
{
  "action": "upvote",
  "concern_id": "existing-concern-id",
  "encounter": {
    "areas": [{
      "relation": "affected",
      "area": {"repository":"example/service","kind":"feature","key":"dispatch","display_name":"Dispatch"}
    }]
  }
}
```

The CLI accepts the same encounter object with `--encounter-json` on
`nexus concerns report TITLE --description DESCRIPTION` or `nexus concerns upvote CONCERN_ID`.
Add an optional pinned `location`, `hypothesis`, `impact`, source evidence or
`observation` metadata when supported by what you observed. If the location is
unknown, supply the known feature or subsystem. A genuinely unlocated report is
valid.
MCP binds the reporter from its connection.
Pass `observation.context.work_session_id` and `observation.context.host_session_id`
when known; otherwise the concern counts as unknown work. The CLI fills omitted
context from `NEXUS_*` variables, a `NEXUS_WORK_CONTEXT` handoff or the active host
bridge; hosts without that handoff, such as Codex shells, need `--work-session`.
The companion binds project/reporter context;
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

## Route a concern to its originating agent

After filing a concern against a feature or pinned Git location, offer an
addressed handoff when the original implementer can help. Use the concern ID
from the receipt.

```sh
nexus delivery remediate --concern "$CONCERN_ID" --title "$TITLE" --message "$MESSAGE"
nexus delivery remediation-status "$REQUEST_ID"
```

The resolver tries a unique pinned location first and can fall back to a
unique recorded feature. Partial concern history prevents automatic pinned
location selection, and dirty locations are excluded. If there is no unique
locator, pass a known origin selector, such as `--repository` with `--feature`
or `--revision`. If several matching origins remain, review their evidence
and use `--agent` with `--session` to select an eligible recipient. See the
[delivery guide](../nexus-pr-management/SKILL.md) for origin qualification, host
presence, and how the recipient acknowledges receipt.

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

The browser opens concerns on the Code map. Keep its project and history range
when comparing the Problem areas and All concerns views. Maps count the selected
measure; unique concerns, raw encounters, and known source occurrences have
different units. Unattributed groups indicate missing recorded dimensions; do
not fill them with guessed feature or subsystem ownership.
