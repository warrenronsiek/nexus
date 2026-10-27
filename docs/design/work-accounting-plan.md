# Closing work-time accounting gaps

Status: implemented Nexus contracts and deterministic adapters, 2026-10-27.
The baseline and acceptance criteria below retain the original plan. The current
contract is specified in [work history](../features/work-history.md); the broader
rationale remains in [workflow history](workflow-history.md).

The implementation adds bounded capture admission with explicit delivery/loss
health, restart-safe duplicate reconciliation, request/turn/child/execution
adapters, exact identity handoff, and qualified accounting. Native and Pi terminal
views share a vertical branching waterfall with time/lane panning and zoom.
Session names follow exact host title metadata when available.

Coverage remains source-dependent. The bundled
[host adapter reference](../../skills/nexus-work-history/references/host-adapters.md)
defines supported events and an isolated real-host verification procedure.
Consumer repositories still own instrumentation and rollout. A missing host
callback, unsupported resume epoch, unavailable capability inventory, or missing
historical source evidence remains an explicit gap. Deterministic fixtures do
not establish that every installed host/version emits every supported event.

## Objective and boundaries

Explain where a work item's elapsed time went across requests, agents, tools,
commands, tests, reviews, and waiting, with traceable source boundaries and
explicit coverage. Preserve nested and parallel execution without double-counting.
Nexus records caller-owned execution; it does not schedule workloads, choose
release policy, or infer task success from a host disappearing.

Keep the implementation organization-neutral. Nexus owns generic lifecycle,
identity, delivery, and query contracts plus supported host adapters. Consumer
repositories and execution services own their scripts, scheduler stages, labels,
and deployment procedures. No consumer names, paths, credentials, or machine
identifiers belong in Nexus source, fixtures, or installation defaults.

## Baseline before implementation

The typed recording API already supports declarations, queues, starts, finishes,
exact elapsed measurements, outcomes, parentage, membership, and corrections.
Automatic coverage falls short of that contract. Adding another generic finish
endpoint alone will not close the gaps.

| Boundary | Baseline implementation | Gap addressed by the plan |
| --- | --- | --- |
| Human request / host turn | Prompt callback adds a milestone to the agent activity. | No independently timed request or turn; no installed stop/interruption adapters. |
| Tool invocation | Pre-tool starts and successful post-tool finishes share a tool identity. Claude also has a failure hook. | Failed, canceled, rejected, or interrupted calls need supported terminal events; partial callbacks remain possible. |
| Host session | First callback opens a root and agent activity. Session-end finishes the agent with unknown success. | The automatically created root stays open; a live long-running session has no request-level completion boundaries. |
| Child agent | Spawn and message calls appear as tools. Explicit environment propagation is available to cooperating callers. | A dispatch return does not measure child execution; automatic child start/stop and causal identity handoff are absent. |
| Commands and named scripts | `nexus exec` measures synchronous execution and propagates context to its descendants. | Ordinary host shell calls do not automatically propagate the companion's work identity into separately launched commands. |
| Asynchronous work | Generic observations can represent a separate execution. | Dispatch, polling, actual process lifetime, retries, and cancellation need a correlated execution adapter. |
| Runner / CI work | Caller helpers and snapshot import can record named intervals. | Producers can record valid, completed runs in unrelated roots when host/parent context is absent. |
| Delivery | Bounded outbox, immutable event IDs, retries, quarantine, and health exist. | Capture can be skipped before enqueue; enqueue errors are discarded; accepted hook responses do not establish durable work-history capture. |
| Accounting views | Known durations, overlaps, counts, and gaps are displayed. | No complete request-level accounting or explicit breakdown of attributed versus unobserved intervals. |

Source anchors for implementation investigation:

- [Host hook generation](../../src/agents/integration.rs).
- [Lifecycle observations](../../src/runtime/work_capture/lifecycle.rs).
- [Capture admission and lifecycle requests](../../src/runtime/client.rs).
- [Outbox enqueue and delivery](../../src/runtime/work_capture/mod.rs) and
  [durable outbox](../../src/runtime/work_capture/outbox.rs).
- [Execution context and measured commands](../../src/runtime/work_capture/activity.rs).
- [Typed work-history model](../../src/work_history/records.rs).
- [Caller integration contract](../../skills/nexus-work-history/references/integration.md).

The code establishes mechanisms that can lose boundaries, not a proven cause
for every historical missing start or finish. Prior oversized-output hook failures
are a separate transport defect; verify the metadata-only completion-hook fix
when integrating this plan. Missing boundaries after that fix require independent
investigation. Legacy history cannot acquire exact measurements without source
evidence.

## Measurement rules

1. A logical work session can span multiple host sessions and delivery stages.
   A host session can stay open while the agent waits for another human request.
   Neither duration is automatically active work or CPU time.
2. Track a human request and its contributing host turns separately when one
   request produces multiple turns or continuations. A turn finish does not
   prove the logical task, child agents, or background processes have finished.
3. Record source start, finish, outcome, occurrence time, and receipt time.
   Prefer exact monotonic elapsed measurements from the executing process.
   A different process retains its own producer identity. Independent callbacks
   may share an explicitly identified OS boot clock; a process-local `Instant`
   cannot be borrowed across processes.
4. Report inclusive execution totals, overlap-adjusted occupied wall time, and
   parallel worker occupancy as distinct measures. Do not sum parent and child
   intervals as total effort.
5. Model-response time means an observed host/provider interval. It is not an
   exact measure of internal reasoning. Uncovered time remains unobserved;
   do not label it idle or model time by subtraction alone.
6. Success, failure, cancellation, timeout, abandonment, and unknown outcomes
   remain distinct. A liveness timeout provides an observation or bound, not an
   invented process exit timestamp.

## Implementation sequence

### Phase 1: make capture loss visible and delivery dependable

Owner: Nexus runtime and work-history capture.

- Reproduce lost-boundary mechanisms with controlled contention, slow durable
  writes, unavailable service, callback bursts, and companion restarts. Distinguish
  missing host emission, rejected host transport, skipped capture, failed durable
  enqueue, pending delivery, permanent rejection, and accepted observation.
- Replace silent capture admission skips and ignored enqueue errors with a
  bounded handoff whose capacity and loss behavior are explicit. Never make the
  workload wait indefinitely or turn telemetry failure into execution failure.
- Retain durable observations before acknowledging capture where the integration
  promises durability. Keep lifecycle-hook permission independent of capture
  acknowledgment; a permissive hook response is not a delivery receipt.
- Expose cumulative loss and rejection reasons, source/host identity, and last
  successful capture/delivery without retaining raw prompts or tool payloads.
  Count pre-outbox loss as well as outbox-capacity loss.
- Preserve event IDs, occurrence times, and producer identity across retries.
  Test duplicate callbacks after the bounded acknowledgment journal is evicted:
  regenerating an old event ID with a new timestamp must not create conflicts
  that interfere with subsequent boundaries.
- Reconcile gaps only from authoritative retained source events. Do not replay
  workloads or manufacture finish times to repair telemetry.

Acceptance: deterministic fixtures account for every offered callback as accepted,
queued, rejected, or explicitly lost; ordinary contention has no unexplained loss.
Restart and exact-replay fixtures retain one logical boundary. Workload output,
exit status, execution count, and host response budgets remain unchanged.

### Phase 2: close request, turn, and host lifecycle boundaries

Owner: generic Nexus lifecycle capture and Codex/Claude host adapters.

- Establish a tested capability matrix for each supported host version. Validate
  actual emitted JSON and tool-name/ID mappings, including code-mode wrappers;
  do not assume one host's hook names or payloads work in another.
- Record timed request/turn activities from supported prompt/turn-start sources.
  Wire normal completion, interruption, and error/cancellation sources. Use
  `Stop` and `Interrupt` where supported; use a supported host event adapter
  when native hooks cannot expose the required terminal event.
- Capture failed/canceled tool completion and preserve tool-use identity across
  pre/post callbacks. Keep tool failure separate from a test reporting failure
  inside an otherwise successful tool invocation.
- Distinguish automatically host-owned roots from explicitly shared logical
  work roots. On observed session end, finish the host activity and close only
  a root whose ownership/lifecycle policy authorizes it. Preserve unknown task
  success. Do not finish unrelated roots or descendants.
- Handle reconnects, resumed sessions, repeated prompts, continuations, and
  duplicate stop callbacks. Treat idle, disconnect, crash, and actual completion
  separately. Any inferred liveness state must disclose its evidence and limits.

Acceptance: real-host fixtures cover two requests in one open session, normal
completion, interruption, failed calls, continuation, resume/reconnect, and abrupt
termination. Finished requests have matched boundaries while the host remains
open; shared roots and independent children remain open appropriately.

### Phase 3: propagate identity and capture child/background execution

Owner: Nexus context contracts and host adapters; consumer launchers adopt them.

- Provide a supported way for shell commands and launchers to obtain the active
  work-session, request/turn, host-session, and immediate parent identities.
  Reuse host-supplied identity where trustworthy; never associate executions by
  checkout path, PID, or nearby timestamps alone.
- Do not assume an MCP hook can mutate the host shell's environment. Implement
  and test an explicit handoff using supported host metadata or a bounded local
  context bridge; preserve explicit caller context over inferred defaults.
- Wire child-agent start/stop observations where available. Give the child its
  own executor identity while retaining its parent and logical work membership.
  A completed spawn call records dispatch, not child completion.
- Assign separate execution IDs to asynchronous commands and actual retries.
  Correlate process start, polls, exit, cancellation, and late completion without
  creating a new execution for every poll. Children may outlive their dispatching
  tool or host turn.
- Add only the smallest typed contract extensions proven necessary by adapter
  fixtures. Keep one lifecycle implementation with explicit source variations;
  do not create a second untyped event store or shell-command parser.

Acceptance: a parent launches a child agent, which launches a measured command;
all three resolve into one work tree with distinct identities. An asynchronous
command survives dispatch/turn completion, is polled repeatedly, then finishes
once. Concurrent sessions in one checkout do not acquire each other's work.

### Phase 4: connect execution services and repository workflows

Owner: each consumer repository or runner; Nexus maintains reusable interfaces.

- Adopt the identity handoff at runner submission, command/review launch, Git
  hooks, and CI submission/import boundaries. Attach existing executions through
  evidence-backed membership/corrections instead of duplicating accepted work.
- Record queue/admission wait, setup, compilation, test execution, cleanup, and
  retry boundaries where the runner actually observes them. Report worker
  occupancy separately from top-level run wall time.
- Preserve terminal outcomes for passing, failed, canceled, timed-out, and
  infrastructure-failed attempts, including runs that never reached a worker.
- Persist source observations for retryable delivery; verify server receipts.
  Recover delivery without running tests, reviews, or commands again.
- Keep consumer-specific commands, scheduler behavior, infrastructure setup,
  and integration patches in their owning repositories. Publish the generic
  identity and lifecycle interface before consumers depend on it.

Acceptance: a real caller workflow records a failed test followed by a retry,
parallel workers, and an asynchronous review under the initiating request.
Queue, compile, execution, and cleanup durations have known coverage. No measured
run is orphaned when the caller supplied a valid work context.

### Phase 5: expose qualified accounting and enforce coverage

Owner: Nexus work-history queries and shared terminal projections.

- Present request/turn completion, session lifetime, measured tool/runner time,
  explicit waits, overlap-adjusted occupied wall time, and unknown/unattributed
  intervals with definitions. Keep partial views visibly partial.
- Surface missing-start/finish rates, unlinked executions, capture losses,
  delivery backlog/rejections, clock conflicts, and source capability coverage.
  A missing source inventory must not become a claim of complete coverage.
- Use interval overlap rules consistently across CLI/MCP and native/Pi views.
  Preserve pagination, truncation notices, project isolation, and exact duration
  text. Bound aggregate queries before materializing histories.
- Add deterministic lifecycle/lineage fixtures to the canonical gate. Add a
  reusable real-host verification procedure for supported adapter versions;
  provider-backed verification remains explicit and outside ordinary CI.

Acceptance: the fixture's known request intervals and parallel execution graph
produce exact expected totals without double-counting. Intentionally missing
boundaries and unsupported sources visibly reduce coverage rather than producing
zero durations or fabricated complete totals.

## Validation and rollout

For each production behavior change, first retain a focused failing regression,
then implement the smallest passing change. Cover both storage backends when
schema or projection behavior changes. Use ordered equivalent SQLite/PostgreSQL
migrations only if typed contracts require new durable state; preserve immutable
history and export/import compatibility.

Run Nexus's canonical `./scripts/check.sh`, applicable PostgreSQL/Compose gates,
and reciprocal staged reviews. Update the owning feature spec and bundled skill
with each shipped behavior. Consumer changes run their own repository gates.

Roll out reliable capture first, then host lifecycle, identity/descendants,
consumer instrumentation, and accounting projections. Reinstall host integrations
and review changed hook trust through the host's supported mechanism. Verify
already-running and newly started sessions without restarting unrelated work.
Use bounded synthetic workloads in isolated repositories; compare offered source
events, delivery receipts, and queried work records. Record the capture start and
adapter version for each coverage period.

Historical repair is separate from rollout. Import or append corrections only
when retained source evidence establishes identity, boundaries, and provenance.
Keep unavailable history unknown; never infer all work in a checkout belongs to
one session. Preserve original observations and explain repair limitations.

## Definition of done

- Supported request, tool, child, and runner fixtures have matched starts and
  terminal outcomes, or an explicit supported reason for an unknown boundary.
- No callback disappears silently at capture admission or durable enqueue.
- Context reaches synchronous and asynchronous descendants without guessing.
- Queue, execution, waiting, and overlap semantics survive failed attempts and
  retries. Measured time is never advertised as CPU use or exact reasoning time.
- Existing and new integrations preserve workload behavior and remain fail-open.
- CLI/MCP and both terminal hosts disclose the same coverage and timing limits.
- Public Nexus remains independent of any consumer's repository or host policy.
