# Work sessions and activity history

Status: approved design, 2026-10-24. The implemented recording contract, capture
coverage, and terminal behavior are specified in [work history](../features/work-history.md).
This document preserves the rationale and broader direction; illustrative
commands below are not the public API reference.

The proposed [work-time accounting plan](work-accounting-plan.md) identifies
remaining capture, completion, correlation, and measurement gaps and sequences
the work to close them.

## Purpose and agreed scope

Nexus should preserve the structure and timing of a coherent unit of work, from
the initial human request through agent work, reviews, tests, pull requests, and
caller-defined delivery environments. The history should explain what happened,
who initiated and performed it, how activities relate, and where elapsed time
went. Callers continue to run commands, manage agents, decide outcomes, and
promote releases.

The primary unit is a **work session**: a logical work item that can span several
prompts, host sessions, agents, machines, repositories, PRs, and environments.
This incorporates the user's confirmation that the work item persists across
sessions and PRs, and their clarification that child and descendant relationships
must be preserved. A host's session ID is one participant's identity within that
history; it does not define the lifetime of the whole work session.

The first delivery should provide durable capture, CLI and MCP access, useful
instrumentation, and a read-only TUI. Later dashboards can use the same query and
export contracts. Timing capture should be deterministic wherever the executing
host or script exposes a boundary; it should require little model bookkeeping.

## The history people should see

The default view is a work session with a nested activity tree and a time axis.
Repeated invocations remain separate children. Subagent work expands into its
own children, including further delegation. A flat list is a secondary search
view.

```text
Work session: Improve router fallback
  Human request received                         milestone
  Implementation agent A                        activity
    Read skill / inspect repository              tool activities
    Test run 1                                   failed
    Worker B: implement fallback                  agent activity
      Inspect code                               tool activity
      Worker C: check edge cases                  agent activity
        Test run                                 succeeded
      Test run 2                                 succeeded
    Commit attempt 1                             failed checks
      Validation                                 activity
        Typecheck / tests / lint                 child activities
    Commit attempt 2                             completed
      Validation                                 activity
      Review group                               activity
        Architecture reviewer D                  concurrent activity
          Reviewer tool calls                    descendants, when exposed
        QA reviewer E                            concurrent activity
    Commit created                               milestone + full Git identity
  Pull request                                   linked external identity
    CI attempt 1                                 failed
      Linux / macOS / other jobs                 concurrent activities
    CI attempt 2                                 succeeded
    PR merged                                    milestone
  Release R                                      linked shared delivery activity
    Deploy to dev / evaluate                     caller-defined activities
    Deploy to beta / approval wait               caller-defined activities
    Deploy to production / verify                caller-defined activities
    Production verified                          milestone
```

The displayed nesting follows recorded ownership/delegation. Parallel activities
remain parallel, and an agent task may continue after the spawning call returns.
The actual release may deliver several work sessions: display its shared link
without duplicating that execution. This example is a possible history, not a
required stage order.

## Core model

Use one new `work-history` feature, with one typed lifecycle for agent tasks,
tools, commands, reviews, hooks, tests, CI jobs, deployments, evaluations, and
explicit waiting intervals.

| Concept | Meaning and essential fields |
| --- | --- |
| Work session | Stable ID, project scope, safe title, creator, creation observation, optional requirement/issue references, and caller-reported completion or abandonment. Can contain work across several host sessions and PRs. |
| Activity | One invocation/attempt: stable ID, semantic kind and name, optional owning work session and parent activity, initiating actor, executing actor, context, and source identity. |
| Observation | Immutable event ID, activity/work-session ID, producer and instrumentation version, occurrence time, receipt time, observation method, and a typed payload. |
| Milestone | A timestamped observation such as request received, commit created, PR opened/merged, environment entered, or production verified. |
| Relationship | An attributable link: parent/child ownership, delegation, retry-of, dependency, shared membership, produced revision, deployed revision, or evidence. Links can arrive after either endpoint. |
| Context | Bound project, work-session/activity IDs, host session/turn IDs, agent instance and model identity when supplied, repository/worktree, and external run/job/deployment identifiers. |

The lifecycle payloads cover declared/queued, started, finished, milestone,
relationship, and correction observations. A completed record imported from an
external system can contain both source boundaries without pretending Nexus
observed it live. Heartbeats may establish recent liveness for long work; they
are optional and never imply a finish.

Activity kind/name and environment identity are caller-extensible. Nexus can
supply convenient names such as `test`, `review.architecture`, `agent.task`, and
`deploy`; it does not require Dev → Beta → Prod or a fixed set of review types.
Use closed typed lifecycle and relation variants, with a bounded named extension
boundary for caller labels and exact text metadata. Avoid a general arbitrary
JSON payload as the core domain model.

Keep these independent:

- Execution outcome: succeeded, failed, canceled, timed out, skipped,
  infrastructure error, or explicitly unknown; optional exit code/signal.
- Report completeness: start missing, finish missing, unresolved links, source
  disagreement, or known capture loss.
- Assessment: for example, a reviewer executed successfully and reported
  findings. A process exit code alone does not establish review approval.

### Parentage, delegation, and shared work

Each activity has at most one structural parent. This produces a navigable
descendant tree rooted in its work session. Parentage is preserved even when the
child uses another agent, host session, repository, or machine. Ending a parent
does not end its children. Ownership does not assert strict timestamp containment.

Agent instance identity, host session identity, and agent-task invocation identity
remain distinct. The task interval starts when dispatch actually occurs and ends
when completion is observed. The brief spawn call and later polling/wait calls
can be separate activities linked to that task. All available child tool/test
entries attach to the task's activity ID. A host exposing only a spawn response
does not establish when the child finished or what happened inside it.

Dependency/fan-in, retry, and shared membership use additional typed links.
A release can include several PRs and work sessions; one CI job or deployment has
one canonical execution identity. Membership makes it discoverable from each
related session without charging its duration multiple times in a global total.
Cross-project links require access to both scopes; opaque external references
remain usable when an internal link cannot be resolved.

The initiating actor and executor are separate. A review started by agent A,
performed by subagent B, and imported by a CI adapter should retain all three
roles. Runtime/authentication binds the submitter; caller-reported actors retain
their assertion provenance rather than becoming impersonated authenticated users.

## Timing semantics

Store source occurrence times separately from Nexus receipt times. Where one
process owns both boundaries, also measure elapsed duration with a monotonic
clock and identify the clock/process instance. Preserve the timing method,
precision when known, source, and any uncertainty. Cross-machine UTC timestamps
support chronology; monotonic readings from different processes cannot be
subtracted. Platform suspend behavior should remain part of the measurement
method rather than an undocumented promise.

| Measurement | Definition |
| --- | --- |
| Work-session lead time | Difference between two explicitly selected milestones, such as initial request → first production verification for the delivered revision. |
| Activity elapsed time | Locally measured duration when available; otherwise a source-reported or wall-clock interval with that basis disclosed. |
| Observed activity coverage | Union of selected activity intervals on a common timeline; concurrent intervals overlap once. Selection and clock limitations remain visible. |
| Aggregate activity duration | Sum of selected activity durations, with an explicit selection such as leaf test runs. Nested wrappers and descendants must not be summed together as total effort. |
| Queue/wait duration | Explicitly observed queued→started boundaries or named waiting activities, such as approval or waiting for a subagent. |
| Unobserved time | Gaps outside the selected observations. These are not automatically human effort, agent idleness, or waiting. |

For example, two parallel ten-minute reviews consume about ten minutes of wall
time and contribute twenty reviewer-minutes to the selected reviewer total.
Neither figure tells us how long the implementation agent was actively working.
Label nested inclusive durations and all aggregate denominators explicitly.

The initial request boundary needs an actual host/caller observation. If capture
starts at a commit hook, show **first observed work → production**, with the
earlier portion unknown. Do not reconstruct prompt timing from session liveness.

Likewise, production completion is caller-defined. Distinguish deployment
requested, deployment completed, and post-deployment verification passed. Retain
the environment, delivered revision/release identity, and selected milestone IDs
so a later query can reproduce what “prompt to production” meant. Superseded,
abandoned, or undelivered work has no successful delivery duration. Rollbacks and
repeated production deployments remain separate observations.

Missing, contradictory, or impossible cross-clock boundaries yield unavailable
or qualified durations, with the original observations retained. Do not clamp a
negative result to zero. Exact critical-path, exclusive CPU time, human labor,
and inferred cost attribution are outside the initial projection.

These operational timing definitions do not constrain the existing flexible eval
metric model. Custom measurements continue to use their versioned meanings,
units, formats, calculations, and caller-owned objectives.

## Recording and reliability

The durable authority is an append-only observation log with indexed
work-session/activity/link projections. Implement SQLite and PostgreSQL through
typed Diesel queries, with new migrations and portable transfer support. Reuse
existing provenance and bounded pagination patterns; preserve domain ownership.

1. Generate IDs before execution or network access. Each new execution attempt
   gets a new activity ID; redelivering its observations reuses their event IDs.
2. Acknowledgment means the server transaction committed. Exact replay returns
   the original receipt. The same event ID with different immutable content is
   a conflict, never an update to its timing.
3. Accept finish-before-start, offline batches, and temporarily unresolved
   parent/context links. Validate scopes, sizes, and known reference types;
   reject immediately known structural cycles. If late resolution reveals a
   cycle or competing parents, retain the accepted observations and expose the
   conflict without following cyclic edges or silently selecting a parent.
   Keep unresolved relationships visible rather than dropping their observations.
4. Preserve conflicting reports with different event IDs. Corrections name the
   superseded report and reason, retain the same subject and compatible event
   family, and have acyclic ancestry. A unique valid correction chain changes
   the projection, including corrected timing or parentage, while preserving
   every original observation. Competing correction branches remain conflicted;
   latest arrival alone does not choose an outcome, finish time, or parent.
5. An open activity stays unfinished/unknown after a crash. Stale/liveness labels
   are separate projections. A later caller can report interruption with its
   evidence. Stopping the agent session does not finish its CI or descendants.
6. Correlate independent observers through explicit invocation/provider keys.
   For GitHub CI, retain repository/provider, run ID, run attempt, and job/step ID.
   Wrappers and host hooks can corroborate one invocation through propagated IDs.
   Similar names and close timestamps are insufficient to collapse observations.

Automatic adapters must remain fail open: telemetry failure cannot turn a passing
test into a failed test, block a commit, change a command result, or decide a
promotion. Preserve the workload's stdio, exit status, and signal behavior.
Direct recording commands return explicit recording errors for callers that
need durable confirmation.

For reliable automatic capture, use one shared client delivery path with a
bounded, private on-disk metadata outbox. Persist events locally before attempting
remote delivery; retry with the same IDs and remove only after acknowledgment.
Report separately **recorded**, **queued locally**, and **lost/dropped**. If the
outbox is full or unwritable, fail open and expose the loss when possible. A
local receipt is not a server receipt, and a crash before an observation was
written cannot be recovered by guessing. Initial outbox behavior needs a hard
byte cap, bounded foreground time, and recovery tests.

The active local companion/client drains its outbox. One-shot hooks attempt a
bounded delivery and leave unacknowledged records for the next active client or
an explicit flush. Expose backlog size, oldest queued age, and loss counters;
offline queuing alone does not promise eventual delivery while no client is
running. The outbox contains metadata only and does not become a new artifact
upload queue.

Permanent server rejections and corrupt local entries move to a bounded rejected
state with safe reason codes and visible counters. Independent valid entries
continue delivery. The byte cap covers pending and rejected entries together;
one undeliverable event cannot block the queue or grow storage without bound.

Retain historical facts through normal backup/export; the seven-day analytics
window and bounded TUI pages must not become retention rules. Index project,
work session, activity/parent, source identity, kind, environment, outcome,
occurrence, and receipt dimensions. Queries use bounded pages and disclose
truncation. Timeline traversal returns children and unresolved links explicitly.

## Deterministic capture and correlation

Propagate a small context envelope containing work-session ID, parent activity
ID, and source/host-session identity. Use inherited environment or an explicit
context file for child processes, and explicit arguments/metadata at agent and
CI boundaries. Context selects attribution; it grants no authorization. A
long-running host binds context per turn/task so concurrent work in one checkout
does not inherit a single mutable “current work session.”

The first request in an unbound host can create a work session automatically.
Later turns continue that binding until the caller explicitly opens/switches
work. Child agents inherit the binding at dispatch. Independently launched hosts
resume through an explicit context or recorded issue/PR/work-session association;
the same checkout, similar prompt, or nearby timestamp is insufficient to join
them. Unbound observations remain visibly unlinked and can be attached later.

| Surface | Proposed capture and limits |
| --- | --- |
| Human input / agent turns | Host prompt callback records request milestone and context, without copying prompt text into timing metadata. First prompt can create a work session; later turns resume or explicitly switch it. Unsupported hosts retain an unknown earlier boundary. |
| Tool and skill activity | Extend existing pre/post capture with occurrence time and correlation. Tool intervals describe the observed invocation. A shell call classified into several operations cannot give each operation an invented execution interval. Skill-read evidence retains its observed meaning. |
| Subagents | Observe actual dispatch/completion and propagate context into child sessions when the host supports it. Caller start/finish is the fallback. Expose missing child internals rather than inventing a complete trace. |
| Repository scripts | Add deterministic start/finish around named steps. Enhance the existing `nexus exec` boundary for callers that choose it, reusing its process handling and inherited context. No second command-launch framework. |
| Git hooks | Track each hook invocation and its named steps. Pre-commit starts a commit attempt with the base commit and staged-tree identity; the new commit does not exist yet. Post-commit or an explicit caller event attaches the resulting full commit. Failed attempts remain visible. Hook installation composes with existing hooks. |
| Nexus commit review | Parent review activity with distinct concurrent architecture/deletion workers; provider execution failure remains separate from review findings and deterministic check outcomes. |
| PR and CI | Caller/CI adapter records PR identity and milestones, workflow attempts, jobs/steps, conclusions, actual tested SHA, and external URLs. Support explicit source imports for reconciliation after runner loss. |
| Environments / delivery | Caller hooks record environment identity, release and deployed revision, deployment interval, approval/queue waits, and independent verification/eval references. No environment ordering is enforced. |

Git integration must preserve repository identity, object format, full commit
IDs, and actual observed/tested revisions. A PR head, a synthetic test merge,
the eventual squash commit, and the deployed revision may all differ. Record
their associations with provenance; do not silently transfer a test result to a
different revision. Before commit creation, the staged tree identifies the
candidate snapshot without manufacturing a commit ID.

Commit hooks require an explicit handoff: when the caller supplies a commit
operation ID before invoking Git, all hooks inherit it and can join the attempt
to the created commit. A pre-commit subprocess cannot export a newly generated
ID back into its Git parent for post-commit to inherit. Without a trustworthy
shared operation ID, retain the attempt and commit milestone with an unresolved
association for later explicit attachment. Do not guess by staged-tree hash or
time; retries can use the same tree. A future hook-only handoff would need its
own worktree-isolated concurrency contract before promising exact correlation.

Hooks alone cannot establish complete Git/PR coverage. Some hooks can be
bypassed, failed commands may never reach a post-hook, and a remote merge or
deployment is not a local Git event. An EXIT trap improves command coverage but
cannot guarantee a report after machine loss or SIGKILL. Source reconciliation
adds evidence; it never fabricates unseen intervals.

## CLI, MCP, and terminal experience

All adapters use one typed recording contract. The delivered CLI includes:

```text
nexus work open / show / list
nexus work start / finish / milestone / link
nexus work record observations.json
nexus work activities / timeline / events
nexus work export / import / schema / health / flush
```

`start` returns an ID or accepts a pre-generated ID. Direct recording commands
require server acknowledgment; automatic capture can queue observations locally
while disconnected. Context supplies attribution, while direct commands permit explicit overrides
within the caller's authorized scope. Finish and milestone calls record facts;
they do not run the workload. A typed `nexus_work` MCP tool exposes the same
operations and schema. JSON output supports scripts and future dashboards.

The shared native/Pi TUI provides a **Work** view:

1. Work-session list: title, elapsed basis, current reported state/outcome, and
   capture gaps. Enter opens the session hierarchy; session detail retains the
   source boundaries and milestone observations.
2. Session detail: expandable activity tree and relative timeline, with tests,
   reviews, agent tasks, and environments as navigable groups. Repeated attempts
   and concurrent siblings remain visible. Crosslinks show shared CI/releases.
3. Activity detail: source start/end and duration basis, initiator/executor,
   session/agent lineage, outcome, original observations, revisions, evidence,
   corrections, unresolved relationships, and missing boundaries.

The delivered list filters cover repository, host session, kind in activity
search, environment, outcome, title search, and occurrence-time bounds. The
dashboard supplies project scope; session membership links navigate back to the
canonical tree. Revision/PR filtering, a latest-milestone list column, selected
activity totals, and interval-union summaries remain future work. Current views
show individual elapsed measurements and their basis without computing aggregate
effort. Later aggregates must label their selected population, inclusive/nested
timing, and capture limitations. Both hosts preserve bounded reads, cancellation,
stale-data notices, keyboard navigation, and terminal sanitization.

## Relationship to existing Nexus features

- **Coordination:** retain host-session and agent identity links. Session
  liveness/claims remain operational state; they do not establish work duration.
- **Usage analytics:** link concrete invocation and capability/category identity
  where known. Categories describe activity; they do not supply missing start/end
  boundaries. Reuse capture adapters without counting an invocation twice.
- **Eval history:** link evaluation runs, definitions, measurements, and artifacts
  to the activity that produced or assessed them. Existing eval records remain
  authoritative for evaluation meaning, motivation, and results. A generic test
  or deployment does not need a fabricated eval suite/target just to be timed.
- **Artifacts:** normal timing capture stores safe semantic metadata and
  references. It does not copy raw prompts, shell arguments, environment values,
  output, or error bodies. Intentionally retained evidence follows the existing
  Git-first artifact policy, with pinned revisions/paths and new bytes only when
  necessary. Existing prompt-storage configuration remains independent.

Related implementation evidence: [usage analytics](../features/usage-analytics.md),
[evaluation history](../features/evals.md),
[terminal UI](../features/observability-ui.md), and
[commit review](../features/commit-review.md). The existing
[`analytics_calls` projection](../../src/persistence/analytics/rows.rs) stamps
receipt time, and its [completion update](../../src/persistence/analytics/mod.rs)
can move that timestamp on redelivery. Those rows cannot reconstruct reliable
historical elapsed durations. The
[`nexus exec`](../../src/runtime/script_exec.rs) wrapper now records work-history
boundaries around execution and preserves its completion-only usage observation.
The [pre-commit hook](../../.githooks/pre-commit),
[check script](../../scripts/check.sh),
[review workers](../../src/agents/reviewer.rs), and
[CI jobs](../../.github/workflows/ci.yml) offer concrete initial boundaries.

## Implementation sequence and review criteria

The approved implementation sequence is retained here for review traceability:

1. **Durable model and recording contract.** Add `work-history` specification,
   typed records/links, SQLite/PostgreSQL migrations, replay/conflict semantics,
   out-of-order ingestion, bounded queries, and portable transfer. Cover the
   complete root → agent → subagent → test descendant structure first.
2. **Caller interfaces and reliable delivery.** Add CLI/MCP, context propagation,
   private bounded outbox, recovery/acknowledgment semantics, and script examples.
   Validate offline start/finish ingestion without controlling execution.
3. **Nexus's own instrumentation.** Capture host boundaries where exposed,
   `nexus exec`, commit attempts, named check steps, and concurrent review tasks.
   Extend setup safely and keep telemetry observational/fail open.
4. **External lifecycle coverage.** Provide a small GitHub Actions/script adapter
   and explicit import/reconciliation for PR/CI source facts, plus generic
   environment/deployment examples. Callers schedule reconciliation; Nexus does
   not become a CI controller or deployment service. Restricted/untrusted CI
   contexts can export metadata for later trusted ingestion without service
   credentials in those contexts.
5. **Shared TUI.** Work-session tree/timeline and source detail in native and Pi,
   followed by clearly defined timing summaries. Exercise a real recorded session
   from initial request through a caller-reported production verification.

Required behavioral checks include:

- Three repeated test runs remain three attempts under the correct parent.
  A subagent's tests and delegated grandchildren are navigable beneath it.
- Two concurrent sessions in the same worktree never silently inherit each
  other's context. Delegation across hosts preserves lineage.
- Duplicate observations do not move timing; retries preserve earlier failures.
  Finish-before-start and late parent arrival resolve without losing facts.
- Crashes, missing completion, source disagreements, and clock anomalies remain
  visible; no timeout/session-stop fabricates completion.
- Parallel reviews and nested wrappers do not inflate work-session lead time.
  Shared releases remain one execution in global totals.
- Pre-commit failure produces no fictitious commit. Tested head/merge/squash and
  deployed revisions retain their actual identities.
- Outbox recovery after process restart preserves IDs and confirms server
  acknowledgment. Permanent rejections do not block independent events;
  full/unwritable queues cannot change workload results.
- Export/import reproduces lineage, original source timing, corrections, and
  unresolved links on both persistence backends.
- The terminal's default session detail reveals the hierarchy and known capture
  gaps without requiring a flat-history search or access to artifact bytes.

Use behavioral test-first implementation and the repository's canonical
`./scripts/check.sh`, plus backend/transport and terminal scenarios appropriate
to each stage. Activity search is a secondary view with links back to the
canonical session tree; filters apply to one invocation. Session-list filters
apply to root attributes. Subject pages expose live projections with an admission
cutoff for new subjects, while immutable event exports freeze their fact set at
the receipt cutoff. A current projection is not an as-of historical snapshot.

Recommended initial defaults: a logical work session spanning host sessions/PRs;
deterministic names and timings; caller-defined completion milestones; existing
Nexus instrumentation plus small caller adapters; metadata-only reliable
delivery; and a tree/timeline as the primary view. Full critical-path analysis,
a general webhook/OTLP collector, continuous provider polling, and graphical
dashboards can wait until capture quality justifies them.

## External reference points

These references inform the proposal; they introduce no runtime dependency:

- [OpenTelemetry tracing overview](https://opentelemetry.io/docs/specs/otel/overview/#tracing-signal):
  parentage, timestamped spans/events, and additional links for causal/fan-in
  relationships. Nexus needs durable open/incomplete observations in addition
  to the familiar completed-span representation.
- [OpenTelemetry tracing API](https://opentelemetry.io/docs/specs/otel/trace/api/):
  ending a span does not complete its children.
- [Git hooks](https://git-scm.com/docs/githooks): hook boundaries, bypassable
  pre-commit behavior, and post-commit/post-merge timing.
- [GitHub Actions variables](https://docs.github.com/en/actions/reference/workflows-and-actions/variables)
  and [workflow jobs API](https://docs.github.com/en/rest/actions/workflow-jobs):
  distinguish workflow run IDs from rerun attempts and retain actual job/step
  source timing and conclusions.
- [GitHub deployments](https://docs.github.com/en/rest/deployments/deployments):
  deployment requests and reported statuses are distinct lifecycle evidence.
- [Rust Instant](https://doc.rust-lang.org/std/time/struct.Instant.html): local
  monotonic measurement and platform-dependent suspend behavior.
