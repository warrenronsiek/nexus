# Recurring code concerns and problem areas

Status: implementation contract, 2026-10-28. The dedicated domain, persistence,
CLI/MCP operations, terminal view and integration skill implement this design.
See the [feature specification](../features/code-concerns.md) for concrete wire
contracts, bounds, provenance rules and retrieval qualifications.

## Purpose

Preserve agents' observations about problematic code across sessions so future
agents and humans can recognize repeated symptoms, failed remedies, and areas
that deserve a broader review. The central question is: **What keeps going wrong
here, what have we tried, and why might another local patch be insufficient?**

Record concrete evidence and distinguish it from suspected explanations. Nexus
organizes the history; callers decide whether to patch, investigate, refactor,
or accept the problem. Reporting does not block work or trigger an automatic
rewrite.

## A separate feature with shared implementation

Code concerns is a top-level feature with its own records, CLI command, MCP tool,
terminal view, and usage skill. The proposed names are `nexus concerns`,
`nexus_concerns`, and feature identifier `code-concerns`. It is not a Papercuts
category, alias, or filtered view of Papercuts records.

The three reporting features serve different purposes:

| Feature | Reporter and purpose | Example |
| --- | --- | --- |
| [Papercuts](../features/papercuts.md) | Development agents reporting ambient development/environment friction. | Memory pressure, orphan test containers, flaky checks, or awkward tooling. |
| [SOS](../features/sos.md) | Agents inside a running application reporting unusual situations outside their expected operating scope. | An email-handling agent encounters an unfamiliar input it cannot handle within its responsibilities. |
| Code concerns | Development agents reporting bugs and recurring problem patterns in the code being maintained. | Several fixes keep returning to the router's fallback logic, suggesting an unresolved design problem. |

Route by the substance of the report rather than the command that exposed it.
A flaky test as an impediment belongs in Papercuts; an identified race in the
application code exposed by that test belongs here. An application SOS report may
motivate investigation that produces a code concern. Link those records rather
than forcing one report to stand for both concepts.

Reuse reporting infrastructure under the hood where its invariants match. Keep
feature meaning and policy explicit; implementation similarity does not make
the records interchangeable.

Keep two concepts distinct:

- A **concern** describes a particular problem or evidenced design concern.
- A **code area** identifies a feature, subsystem, file, or declaration. An area
  can attract several different concerns; one concern can affect several areas.

A report may also link an existing papercut, SOS source, eval result, or work
activity as supporting evidence. Linking the same failure through several
features must not manufacture additional encounters. SOS retains its application
report/triage contract; work history retains execution, timing, and session
ancestry. Cross-feature evidence links do not consolidate records or transfer
lifecycle state.

## What to retain

| Record | Essential information |
| --- | --- |
| Concern | Its own project-scoped concern identity, title, concrete problem description, optional broader hypothesis and impact, creator, and lifecycle history. A one-off concern is valid before recurrence is known. |
| Code area | Project/repository-scoped stable ID, kind (`feature`, `subsystem`, `file`, `declaration`), display name, and attributable containment/membership links. A class, function, method, module, or type is a declaration with a recorded kind. |
| Encounter / upvote | Stable event ID, concern ID, source occurrence, reporter observation and receipt timestamps, bound reporter, host session, optional logical work session/activity, source failure identity, location snapshots, symptom, consequence, and any workaround. The initial report creates the first encounter. |
| Location snapshot | Repository identity, full Git object ID and object format when known, repository-relative path, optional language and qualified declaration name/kind, optional range, and explicit availability/verification state. Feature/subsystem-only reports remain valid. |
| Remedy / review note | Concern and supporting encounter IDs, what was tried or considered, why, affected revision/PR/work references, reported outcome or uncertainty, and author/time. Broader review conclusions and rejected alternatives remain retrievable. |
| Lifecycle change | Resolution, reopening, consolidation, or correction with reason, actor/time, and optional fixing revision and verification evidence. History is retained rather than overwritten. |

Transport supplies project and submitter provenance. Host-session identity and
logical work-session identity remain separate; a subagent is not an independent
work item merely because it has a different host session. Missing context is
explicitly unknown. Imported evidence retains its original claimed source and
the current importer separately.

An agent should only need a short description and its known location to report.
Nexus attaches available session/activity/repository context automatically.
Missing locations remain unlocated and searchable; no up-front component catalog
or whole-repository index is required. Agents can report suspected maintainability
problems, but must label them as hypotheses rather than observed failures.

## Code identity across time

Use feature identifiers from a repository's annotations/catalog when available;
other repositories can supply their own feature and subsystem names. These names
are labels, not global identity. Areas may have multiple memberships: a shared
declaration can participate in several features. Do not force every repository
into one strict feature-to-class hierarchy.

Resolve supplied locators within project, repository, and area kind. Exact
feature/subsystem keys or file paths can reuse an area; declarations additionally
need a qualified name and signature/discriminator where ambiguous. A bare class
name does not establish identity. Historical rollups use memberships recorded
with the encounter; later mappings and corrections retain their attribution and
must not silently regroup old reports.

Keep the logical area ID separate from its historical locations. Commit, path,
and qualified declaration identify the reported code; line numbers are optional
navigation hints. A rename or move can add an explicit, attributable mapping.
Ambiguous matches remain suggestions, not silent reassignment. Deleted or split
declarations retain their original evidence and can link to successors without
pretending they are identical.

Area links distinguish **affected/observed here** from **suspected cause here**.
A failing test location alone does not prove that the test implementation caused
the defect. Supplied, mechanically verified, and model-suggested mappings remain
distinguishable. Correcting a mapping appends a reason and preserves the old one.

Prefer Git references to copied source. Dirty/untracked code records the base
commit and dirty status; attach an explicit diff or content-addressed artifact
only when needed to retain the relevant evidence. A base commit must not be
presented as the exact dirty file. Missing Git objects or artifacts remain visible
as unavailable. Reuse Nexus's Git-first artifact policy without requiring a fake
eval run to attach evidence.

## Encounters, votes, and recurrence

For this feature, define **upvote = encountered this problem again**. Reading
an earlier report and agreeing with it is not a new occurrence. Add context to
an existing encounter by appending attributed evidence when reporting more
details about the same failure. Do not overwrite the original encounter or add a
vote merely to attach detail. Retain raw observation counts and derive the
deduplicated counts separately. This is an explicit concern-domain rule, even
where a shared implementation also supports Papercuts encounters.

Every new encounter has a caller-generated event ID. Identical retries return
the original receipt; changed content under that ID is a conflict. A genuinely
new failed execution gets a new source identity. Where several agents reference
the same failed test attempt, use that source identity to group their observations
as one occurrence with several reporters. Qualify it by source/repository,
execution attempt, and failure identity; a whole CI job can contain several
independent failures. Group occurrences within a concern, while area totals also
deduplicate shared sources across linked concerns. Missing source identity stays
disclosed; Nexus cannot guarantee independence from prose alone. Imported concern
observations lacking this evidence retain unknown deduplication quality. Existing
Papercuts votes remain Papercuts records, not implicit concern history.

Report these separately: raw observations, distinct known source occurrences,
distinct logical work sessions, distinct host sessions, and unknown-context
counts. This prevents one agent retrying thirty times, or thirty subagents reading
one log, from looking like thirty independent sessions. A derived area rollup
counts each concern/encounter once within that area, even through several paths;
overlapping areas are not additive totals.

Exact matching for code-concern reports includes structured area identity as
well as normalized text, within this feature and project. Identical wording in different code
areas must not silently consolidate. Sharing an area is also not evidence that
two problems are duplicates. Semantic consolidation requires an explicit,
attributable approval by the source reporter and preserves original evidence;
a model may propose a match but cannot approve it. Reuse the existing approval
mechanism if these rules match, with concern-typed targets that cannot name a
Papercuts or SOS record. Explicitly selecting an existing concern permits another
encounter at a different location.

Resolving a concern requires a reason; a workaround or a successful command alone
does not establish that the underlying problem was fixed. Record attempted
remedies without requiring resolution. A later encounter reopens a resolved
concern. An imported observation known to predate resolution adds historical
evidence without reopening it. For code concerns, use the failure's source
occurrence time, not when a later agent read its log; unknown ordering records
an unresolved lifecycle qualification without automatically reopening. These
are Code concerns' recurrence rules; existing Papercuts and SOS behavior is not
changed by sharing implementation primitives.

Show whether the reported revision contains the recorded fix, predates it, is on
another branch, or has unknown relation. A recurrence is not automatically a
regression: an encounter on a branch without the fix cannot establish that the
fix failed. Accept supplied or locally verified ancestry evidence; no automatic
Git fetch or repository index is required. Fix containment itself does not prove
a causal explanation either.

## Agent workflow and retrieval

Expose a top-level `nexus concerns` CLI command and a dedicated `nexus_concerns`
MCP tool, with location filters and typed operations to report, upvote, attach
evidence, record a remedy/review, resolve, and read. These commands operate on
concern IDs and do not require callers to use Papercuts or SOS APIs.
Add bounded concern detail/history and code-area summary queries. The complete
concern dump/export must preserve all locations, encounters, mappings,
remedies, lifecycle events, and duplicate decisions.

The intended agent flow is:

1. Query concerns for the feature or code area being worked on, including recent
   resolved concerns and attempted remedies. Read supporting evidence as needed.
2. Report a new problem, or upvote a matching concern with the current encounter
   and source identity. Continue the original task.
3. Record the remedy and why it was chosen. If the history suggests a broader
   problem, add a review hypothesis and links to the evidence.
4. Verify and record resolution when justified. Future agents can see the entire
   chain, including approaches that did not work.

Read queries disclose their project, time window, filters, and completeness.
Ordinary pages are bounded with continuation cursors; aggregate counts cover the
selected population rather than just the loaded page. A truncated result cannot
establish absence.

## Human view

Add a separate read-only **Code concerns / Problem areas** tab to the native and
Pi TUI, alongside Papercuts and SOS. Start with an overview of recurrence across
time, then let the user select an area or concern.

- Area overview: ranked bars, recent activity sparkline, open concern count,
  distinct affected work sessions, encounter counts, and last observed time.
- Area selection: breakdown by feature/subsystem/file/declaration, including
  shared or unresolved memberships, and the distinct concerns contributing to it.
- Concern selection: timeline of encounters, remedies, resolutions, and returns;
  metadata below the selection shows evidence, session titles, and Git locations.

Offer explicit sorts such as affected work sessions, recent encounters, or
returns after resolution. Any configurable recurrence threshold is an advisory
review signal for a follow-up, not a first-version requirement or execution gate.
Label the window and counts; reported code problems are not an objective defect
rate, architectural score, or proof
of causation. No reports means **no reported problems**, not verified health.

For example: a router subsystem has nine observations across five work sessions,
covering three different concerns. One declaration has returned twice after
recorded remedies. Its timeline explains which fixes were present, why each was
tried, and why a broader design review was suggested.

## Implementation boundaries and acceptance

Use typed domain records and Diesel queries for SQLite and PostgreSQL, with
append-only evidence and rebuildable summary indexes. Keep the code-concern
domain, record identities, and public APIs distinct. No Papercuts or SOS records
are migrated or reclassified by default, and historical symbols are not inferred
from old prose without attributed enrichment.

Share cohesive implementation primitives where useful: project/provenance
binding, immutable observation receipts, timestamped encounters, audit events,
duplicate-proposal authorization, bounded pagination, transfer, Git/artifact
references, and terminal rendering components. Reuse requires matching semantics;
for example, existing Papercuts upvotes are not idempotent source observations,
and SOS model triage has different consolidation authority. Do not silently give
either feature new rules to fit a common abstraction.

Use explicit feature-specific types or typed adapters for commands, subjects,
matching/triage policy, recurrence, and projections. A shared database mechanism
must retain feature-qualified identity and prevent cross-feature mutations.
Extract reusable pieces while implementing the feature; a wholesale rewrite of
the other reporting systems is not a prerequisite.

Persist reports independently of provider availability. Optional model assistance
can suggest scope mappings, duplicates, and cross-report hypotheses; it cannot
silently turn them into verified facts or authorize consolidation. Automatic
capture remains advisory and fail-open, preserves command results, and avoids
polling-generated duplicates. Explicit ingestion failures remain visible.

Keep automatic capture limited to safe metadata. Intentional reports contain
bounded, sanitized evidence; never collect full prompts, raw environments, or
whole checkouts by default. Stored source/model text remains untrusted data.

Acceptance scenarios:

- Several sessions encountering the same concern reveal its full history and
  distinct-session recurrence; delivery retries do not inflate it.
- Distinct problems in one subsystem remain distinct and contribute to the same
  area overview; overlapping memberships do not double-count its totals.
- A class rename, dirty file, missing commit, and unknown session preserve useful
  reports with explicit uncertainty rather than invented precision.
- Resolution, a later encounter, and late historical evidence produce the correct
  lifecycle and disclose whether the fixing revision was actually present.
- Several reporters of one execution retain their observations without creating
  several known source occurrences. Imported or missing-source uncertainty
  remains visible.
- CLI/MCP retrieval, both terminal views, and portable transfer preserve the same
  scoped evidence and work with no model provider configured.
- The dedicated command, MCP tool, and TUI view expose only code concerns;
  links to Papercuts/SOS are explicit, and mutations never cross feature domains.

The implementation includes typed storage and idempotent recording; CLI/MCP and
work/eval/SOS evidence links; area/recurrence terminal views; and the installed
`nexus-code-concerns` skill with integration examples. The Nexus routing,
observability and reporting guides explain the feature boundaries. The feature
specification and source annotations identify the owning entry points.
Automatic whole-codebase indexing, inferred root-cause verdicts, and automatic
refactoring are outside this implementation. Configurable review thresholds and
automatic bounded context injection can follow once recording and retrieval are
useful.
