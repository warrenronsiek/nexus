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

## Verify and hand over

Exercise a representative nested workflow and a real failed attempt. Check
parentage, distinct retry IDs, source boundaries, outcomes, and links. Verify
that unavailable Nexus capture does not change or repeat the workload. Validate
queued-versus-acknowledged state separately.

Inspect roots with `work list`, invocations with `work activities`, and immediate
children with `work timeline`. Follow cursors; use `show` and `events` for source
observations, conflicts, and gaps. Root filters apply to root attributes; use
activity search for task-kind filters. Native and Pi Work tabs open a duration
dashboard: qualified session totals/means, activity-kind bars, and daily trends.
Choose a parent session to see every retained descendant on one visual time
axis, with nested and parallel intervals preserved. Use `m` for mean/total,
`t` for session/activity trends, and `[` / `]` to choose an activity kind. The
`d` source browser retains filters, hierarchy, links, and original observations.
Dashboard samples are capped and explicitly partial when a history or display
bound applies. A mean uses timed, conflict-free samples; an unknown interval is
not zero. Inclusive sums are not total effort or prompt-to-production lead time.

Hand over root and important activity IDs, instrumentation locations, covered
sources, delivery recovery, and what remains unknown. Explain what the evidence
establishes while leaving the caller in control of its workflow.
