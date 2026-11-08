# Execution-flow audit: Clarify session time fork charts

The reported problems span capture, projection, and layout. This audit used the
installed viewer, its metadata-only work API, local capture health, the exact
Codex session log, and a private consistent SQLite snapshot. No production work
observations were rewritten, and no raw command arguments or outputs are added
to this report or implementation.

## Confirmed findings

- Initial selected-session read: 443 activities, including 334 titled `Bash`,
  33 missing finishes, two missing starts, and no invocation-mode metadata or
  projected returns. Twelve completed container intervals are distinct from
  completed tool invocations.
- Exact native item IDs matched 333 recorded shell calls. Of the missing-finish
  shell records, 26 already had completed source items: 14 successful and 12
  failed commands. All 26 source items had completion timestamps. These are real
  capture gaps, not evidence of commands still running. Their source process
  intervals and Nexus callback intervals must not be combined without an explicit
  source reconciliation contract.
- Capture health reported 460 cumulative losses (395 writer-unavailable and 65
  capacity), 126 conflict rejections and no pending queue. These are machine-wide
  historical counters, not an attribution of those 26 missing completions.
- Existing MCP processes started before the installed update. Replacing the
  executable does not replace code already loaded by those companions. The
  package version alone does not identify the exact capture revision.
- Tools already stores concrete operation/category/evidence separately. Work's
  prior projection only consumed newly added operation milestones, so it ignored
  usable historical classifications.
- The service currently ignores optional analytics enrichment errors. Missing
  analytics rows cannot be reconstructed from the original generic Work name.
  This is a reachable loss path; this audit does not claim it explains every
  historical missing row.
- Missing-finish execution rectangles occupied the entire remaining session in
  collision packing. Fixed 180px child offsets and a 1000px time axis amplified
  label crowding on multi-hour sessions.

## Implemented changes

Work detail and timeline now reuse Tools' stored operations through exact
project/host/session/invocation identity. Concrete operation names become primary
labels; activity buckets and classification evidence remain available. No new
classifier runs on reads. Generic native tool callbacks carry synchronous
invocation semantics even before the explicit milestone was introduced; caller
work and background worker completion do not imply a synchronous return.

Future native captures also retain up to three safe deterministic operation and
category pairs in a separate durable milestone. This shares the existing resolver
and work outbox, preserves replay compatibility, and survives an optional
analytics write failure. Richer stored Tools classifications take precedence.

Unobserved execution finishes use open boundary markers and explicit labels,
without reserving a session-long duration. Child offsets start at 72px and grow
for actual collisions. Dense sessions get additional vertical source-time space;
UTC remains proportional. Compact intervals bound operation detail separately
from full activity detail and disclose truncation.

A same-input 481-node layout comparison, retaining generic labels to isolate
geometry, reduced width from 14,030px to 2,726px. The candidate uses 19,240px of
vertical source-time space rather than compressing the entire session into
1,000px. Browser measurements and final source-projection results are recorded
below after validation.

## Remaining evidence limits

Historical missing source completions are not repaired with analytics receipt
times, elapsed estimates, or later task starts. Reconnecting companions enables
new capture behavior but cannot recover old losses. A source-log reconciliation
adapter remains separate work: it must retain source clocks and identities,
privacy, explicit correction provenance, and reliable retry/deduplication. The
26 matching completed source items provide concrete regression cases for that
adapter; no workload needs to be rerun.

A separate source-log audit found 352 completed `Reasoning` items with ordered
source start/completion timestamps, while the Work projection has no reasoning
activity kind. These are source item intervals, not proof of CPU utilization or
continuous model computation. The current hook surface does not record them.
This explains why uncovered agent time must remain unknown today; reliable native
source ingestion is required before those intervals can be shown as observed
reasoning activity. Source reasoning text was not retained by this audit.

## Snapshot verification

The later frozen snapshot contains 481 activities (including calls made during
this audit). Running the release and candidate against the same snapshot gives:

| Observation | Released | Candidate |
| --- | ---: | ---: |
| Retained timeline activities | 481 | 481 |
| Generic `Bash` labels | 371 | 94 |
| Qualified tool return paths | 0 | 433 |
| Source timestamp / elapsed changes | 0 | 0 |
| Display population truncated | No | No |

327 candidate activities have correlated operation metadata. Remaining generic
shell labels have no retained operation metadata to reuse. Missing starts and
finishes stay explicit. The candidate response is approximately 598 KiB, inside
the unchanged overall transport cap; compact operation details disclose their
own truncation independently of timeline coverage.

Duration labels now prefer reported elapsed nanoseconds. An interval whose
execution timing is not qualified is labeled `duration unqualified` rather than
assigning a duration by subtracting unrelated UTC boundaries.

Regression evidence was observed red before green for exact operation/category
reuse, durable classification without analytics, missing-finish geometry,
closer child placement, dense source-time layout, compact operation bounds,
timeline response budget, category labels and measured-duration labels. Further
negative tests retain project/invocation isolation, unknown caller-authored call
modes, and missing finishes despite completed analytics records. Browser checks
at desktop, 768px and 390px widths keep all horizontal overflow in the chart's
own viewport. The source-based reasoning and missing-completion limits above
remain unresolved; the preview must not be described as complete capture.

Validation: the canonical `./scripts/check.sh` completed successfully, including
748 Rust tests (one opt-in test ignored), SQLite and PostgreSQL Clippy, feature
annotations, structural checks and the frontend gate. The final duration-label
formatter changes were followed by a fresh complete `scripts/check-web.sh`
(26 Vitest and 13 Elm tests plus deterministic production build), feature lint,
and pinned-BCA structural check. Served preview HTML/JS/CSS bytes exactly match
the final generated bundle. Taste lint has zero failures; its seven monospace
warnings concern the existing numeric/code typeface. No PR, merge, production
restart or historical correction is part of this audit follow-up.
The final browser renders the 481 activities plus the session envelope (482
selectable nodes), 433 return paths, and a 3,030px-wide by 19,280px-high chart.
The 1,280px page has no horizontal overflow. The selected real `rg` call shows
`code search (deterministic)` and its reported 0.3s duration. The read-only preview
runs on port 7350 against the private snapshot; port 7338 remains the installed
release.
