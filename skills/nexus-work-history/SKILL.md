---
name: nexus-work-history
description: Integrate agent, test, review, CI, and delivery workflows with Nexus work history. Use when implementing instrumentation, wiring scripts or hooks, or tracing nested work across host sessions and PRs.
---

# Record work without taking over its execution

Nexus is a durable repository of what was tried, why it happened, who did it,
which work it belonged to, and the observations and evidence behind its result.
The goal is to reconstruct request-to-production work and understand where time
went, including repeated tests, parallel reviews, agent descendants, failures,
and delivery environments.

The caller owns execution, scheduling, retries, task definitions, success
criteria, and deployment policy. Nexus does not choose an optimization target,
run an eval through its recording API, or decide that a release should advance.
Use task kinds and environment labels that describe the caller's workflow. Link
eval records for measurements and their definitions; do not turn one metric,
pipeline shape, or provider into a Nexus requirement.

Read [the integration reference](references/integration.md) when implementing
scripts, hooks, adapters, or direct observations. It covers the installed
helpers, exact command shapes, identity handoff, failures, and verification.
Use `nexus work schema` and the installed command's `--help` as the current API
contract. Prefer existing host capture and named boundaries over adding manual
recording to every tool call.

For native request/turn/child callbacks, active identity handoff, asynchronous
execution events, session names, and source capability verification, read
[host adapters](references/host-adapters.md). A host callback permission response
and a durable capture acknowledgment answer different questions.

## Preserve the work graph

- Reuse one logical work-session ID across the prompts, host sessions, agents,
  PRs, and environments contributing to the same work. A host session is a
  participant, not the definition of the logical work's lifetime.
- Give each concrete execution its own activity ID. A second test run or retry
  is another attempt, linked to its predecessor with `retry_of` where known.
  Keep the earlier failure and its timing.
- Preserve one structural parent for nested work. Propagate work-session and
  parent-activity context when dispatching a child. Record descendants under
  that child rather than flattening them into the original session.
- Explicit work/parent context takes precedence over an inherited handoff.
  Native hooks retain their supplied identity; command launchers can fill absent
  context from a handoff. Active-host defaults retain a bound host and supplied
  executor. Treat malformed handoffs as capture gaps, without inventing identity.
- Keep shared CI or release execution canonical once; use additional `member_of`
  links for other work sessions. Dependencies, delegation, evidence, and produced
  or deployed revisions are separate relationships, not reasons to reparent it.
- Record actual boundaries. A spawn acknowledgment is not a completed child,
  a parent finishing does not finish descendants, and a host disconnect does
  not establish task success. Preserve missing outcomes.

Related repositories need the same Nexus project binding to resolve one tree.
Different project namespaces retain external references unless evidence is
explicitly imported. A project ID is a namespace, not a credential boundary.

## Make capture useful and honest

Use deterministic source callbacks or `nexus exec` for known command boundaries.
Retain source identity, occurrence time, measurement method, actor/model context,
and causal links. Record a task's purpose in bounded descriptive metadata or a
milestone, and link the requirement, eval, incident, or revision explaining it.
Do not invent a reason retrospectively.

Separate occurrence time, Nexus receipt time, and measured elapsed duration.
Prefer the wrapper's monotonic measurement for synchronous commands. Separate
CLI `start` and `finish` processes do not establish a shared clock: supply a
duration measured by the owning process when available, with its method and clock
identity. Keep decimal nanoseconds as text. Do not subtract unrelated clocks,
clamp anomalies, or add inclusive parent/child durations as total effort.

Automatic telemetry must execute the workload exactly once, preserve its stdio
and outcome, and fail open when recording is unavailable. Queued metadata is
not a server acknowledgment. Inspect `work health` and use `work flush` for
delivery recovery; never recover telemetry by rerunning the workload. Direct
`work record` or MCP recording requires acknowledgment and should report
ingestion failure to the integration requesting it.

Persist exact observations with stable event IDs for transport retries. Exact
replay is idempotent; changing an accepted event under the same ID is a conflict.
Convenience commands generate new observation IDs on each call, so rerunning
them is not a transport-retry mechanism. Append typed corrections with their
reason instead of rewriting history.

Automatic capture stores metadata, not prompts, raw arguments, environment
values, outputs, or secrets. Link deliberate evidence through the
[eval artifact contract](../nexus-eval-history/references/nexus-evals.md): prefer
repository/full Git object ID/file path when Git retains the
bytes; use explicit blob evidence when it does not. Report gaps rather than
claiming complete visibility into unseen agent internals.

## Verify capture

Exercise a representative nested workflow and a real failed attempt. Check
parentage, distinct retry IDs, source boundaries, outcomes, and links. Verify
that unavailable Nexus capture does not change or repeat the workload. Confirm
which observations were queued and which had a receipt.

## Read recent sessions and source records

Inspect roots with `nexus work list`, invocations with `nexus work activities`,
and immediate children with `nexus work timeline`. Follow cursors. Use
`nexus work show` and `nexus work events` for observations, conflicts, and gaps.
Root filters apply to root attributes. Apply task kind filters to activity reads.

In the authenticated Work history view, select the project and time range
(24 hours, 7, 14, or 30 days), then choose a harness session. The picker
defaults to seven days and the URL retains the chosen scope. An exact link
still opens an older session and its observed descendants. Before selecting a
session, Where time goes summarizes elapsed time by activity kind across the
sampled roots.

The `dashboard` action on `nexus_work` defaults to all roots. Use
`session_origin: "host"` for the browser's harness scope, `occurred_after`
with a UTC timestamp for a source-time cutoff, and `work_session_id` to
retrieve one session's descendants. The host overview omits sessions that
only recorded containers (`host.session`, `request`, `host.turn`);
`hidden_session_count` reports the omitted roots. Source reads retain
standalone commands and shared logical work. The host sample prioritizes
whole recent sessions, with its size in `activity_session_count`. Each
row's `active_seconds` is known when its human request intervals are
timed and conflict-free. Host lifetime includes time between prompts.

In the scrollable execution tree, time runs down and individual activities
branch right at their observed boundaries. Use Fit, zoom, or pan to inspect
dense sessions. Select a call for its label, kind, elapsed time, outcome,
and correlated operation evidence. Read `show` or `events` for exact source
times, executor, parentage, and capture gaps. Session names come from host
metadata when available. Otherwise host and worktree identify the session.

Read truncation notices before interpreting aggregates. Means use timed,
conflict-free samples. Inclusive durations can overlap. Worker occupancy,
qualified elapsed time, and CPU time measure different things. Use
`nexus.accounting.role=container` for enclosing spans. Explicit execution and
wait intervals with the exact executor can qualify parts of a container.

Keep an unavailable model unknown; empty optional host model hints normalize to
absence. A successful advisory hook is not a delivery receipt: inspect
`nexus work health` and the request's completion and elapsed fields. After fixing
an invalid-capture cause, preview `nexus work retry-invalid --dry-run`, explicitly
requeue a bounded batch with `nexus work retry-invalid --limit 128`, then run
`nexus work flush` and inspect health again. This uses only the configured outbox
and preserves source payloads and identities. Conflict/corrupt quarantines and
other namespaces are excluded. Do not rerun the workload or invent missing times
to fill historical gaps.

Blank optional child-agent hints are absent, so root-agent tool capture retains
the known host executor and turn parent. Retained blank executor hints decode as
unknown during recovery; source identity, time and parentage are preserved.
Validate tool activities as well as request/turn/session timing when checking a
host integration. Missing historical executor or parent evidence remains visible.

Session stop attempts to drain final work delivery within its existing response
deadline. Check completion in every visited project's namespace; a tool run in
another checkout may be recorded there. Verify automatic exit delivery before
using an explicit flush, so manual recovery does not conceal a shutdown gap.
Check the logical work-session root as well as its host activity. Compatible
records use transactional batch delivery; a local pending receipt can still
represent an uncertain acknowledgment. Confirm the exact canonical observation
before treating such a receipt as already delivered.

Use `nexus work flush --limit 256` for an oversized retained backlog. Each
explicit recovery pass holds at most 1 MiB of payloads and attempts delivery
for up to ten seconds. Repeat and inspect health. Automatic hooks retain their
short delivery budget. Local health includes every retained pending boundary,
including legacy oversized backlogs.

A named legacy harness root may predate host metadata. Nexus recognizes its
original companion producer and exact recorded host-root identity; importing
retains the original actor for this check. Standalone command roots remain in
source browsing. Names alone never establish host ownership or missing parentage.

For execution-flow inspection, filled ribbons mean qualified observed execution,
dotted hollow ribbons mean waiting/queued, and grey hollow dashes mean unknown.
Native tool callbacks record synchronous invocation returns; a background worker
retains its independent lifetime. Direct adapters can label an activity declaration
`nexus.flow.mode=synchronous` or `asynchronous` and record a `Result consumed`
milestone on the asynchronous activity when its result is actually consumed.
Do not use worker completion as an inferred join. Record explicit execution/wait
intervals with the exact executor to qualify container state; never label the
gaps between tool calls as model reasoning without source evidence. The chart's
Fit/zoom, independent call labels and return paths are visual projections only.

Work's concrete operation names, activity buckets, and classification evidence
reuse Tools through exact invocation identities, including historical native
callbacks. An open marker with `finish unobserved` indicates missing source
completion; it is not a long-running measured task. Compare `work health` and
native host evidence when auditing these gaps. Analytics receipt times cannot
repair missing source execution times. After updating Nexus, reconnect existing
MCP companions so new callbacks use the updated capture code.
New capture retains safe deterministic operation/category pairs in the durable
work outbox independently of optional analytics enrichment. The compact timeline
shows at most three operations with a truncation gap; full activity detail keeps
all correlated operations. Dense sessions get more vertical source-time space.

For a slow Work read, inspect the installed service version, query plan, and
readiness logs. V30 indexes the candidate source-time window before projection.
Earlier migrations V24–V27 index operation enrichment, event counts, membership
and subject kinds. Explicit store acquisition has a bounded wait. Treat an
unavailable response as a failed read and retain the observation history when
upgrading.

Hand over root and activity IDs, instrumentation locations, verified source
coverage, delivery recovery, and remaining evidence gaps.
