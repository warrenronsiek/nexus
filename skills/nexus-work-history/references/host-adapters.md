# Host boundaries and explicit context

Nexus records observed boundaries. It does not turn tool dispatch into child
execution, host disappearance into success, or uncovered time into model work.
Commands and runners retain their own scheduling, retries, and outcomes.

## Supported source contracts

The adapter revision is `host-hooks-v1`; compatibility fixtures exercise the
published Codex CLI 0.162.1 hooks contract and Claude Code 2.1.280 payload shapes. Codex
native tool-hook inputs omit the optional child `agent_id` so main-thread events
can render; dedicated child lifecycle callbacks retain child identity. They are
synthetic contract fixtures, not a claim that live hooks were enabled or emitted
every callback. Check hook trust and run the verification below after installing.

| Source | Codex | Claude Code |
| --- | --- | --- |
| Request/turn start | `UserPromptSubmit` with `turn_id` | `UserPromptSubmit` with `prompt_id` |
| Response boundary | `Stop` | `Stop` |
| Explicit interruption | `Interrupt` on the main thread | No generic installed interrupt source |
| API error boundary | Caller event adapter | `StopFailure` |
| Child responding interval | `SubagentStart` / `SubagentStop`, exact `agent_id` | Same; resumed agents require a distinct caller attempt ID for separate attempts |
| Tool result | `PostToolUse`; tool success is distinct from command exit status | `PostToolUse` / `PostToolUseFailure` |
| Session lifetime | `SessionStart` / `SessionEnd` | Same |
| Actual asynchronous process lifetime | Caller execution events / `nexus exec` at worker | Same |

Source references: [Codex hooks](https://learn.chatgpt.com/docs/hooks) and
[Claude hooks](https://code.claude.com/docs/en/hooks). Host versions and surfaces
can omit fields or callbacks. An absent turn identity produces a coverage-gap
observation, not an ID derived from prompt text. Raw prompts, model responses,
tool arguments/results, and errors are not retained by this work adapter.

A stop is an observed response boundary with unknown task success. It closes its
request/turn envelope, leaves the host open, and does not close children or a
shared work root. An observed session end closes the host-owned default root
only; a caller-supplied logical root retains its caller-owned lifecycle. Abrupt
termination supplies no finish time. Continuations with distinct source turn IDs
get separate intervals. A host reusing the same agent ID cannot prove distinct
worker attempts unless its adapter supplies an execution/attempt identity.
`SessionStart` with `source: "resume"` marks missing host execution epochs as
partial coverage. A resumed source that reuses its session ID does not acquire
invented new start/finish facts. Preserve actual runner attempt IDs explicitly.

## Identity reaches launchers explicitly

Hooks cannot mutate their parent process environment. Claude's `SessionStart`
command adapter uses the host-provided `CLAUDE_ENV_FILE` export surface to pass
its exact agent and session identity to later shell commands. Runners can resolve
current parent context with `work context`; they must not freeze a startup turn.
Codex shell commands can use their exact `CODEX_THREAD_ID` with `--agent codex`.
When no supported host identity is available, obtain a handoff explicitly, carry
it to the launcher, and let the launcher propagate it to workers:

```sh
nexus work context --agent codex --host-session "$host_session_id"
export NEXUS_WORK_CONTEXT='{"format_version":1,"work_session_id":"...","parent_activity_id":"...","request_activity_id":"...","host_session_id":"...","agent":"codex","turn_id":"..."}'
nexus exec --kind test --name 'Unit tests' -- ./scripts/test.sh
```

The exact project/agent/host-session bridge retains the observed work root,
request, turn, and immediate parent. It has a private 16 KiB limit per host,
4,096 host entries overall, and sixteen recent turn-to-request bindings. The
project key uses Nexus's existing Git project binding or configured remote
project. `--turn` can supply an exact source turn; an uncaptured host without a
supplied turn is an error. Caller request envelopes can span `TurnStart` /
`TurnFinish` pairs; `RequestFinish` closes the independent request. A `Stop`
records the response boundary of that request.

Use the returned JSON verbatim; do not fill IDs by inspecting nearby activity,
checkout paths, PIDs, or timestamps. Explicit `--work-session` / `--parent` flags
and explicit `NEXUS_WORK_SESSION_ID` / `NEXUS_PARENT_ACTIVITY_ID` take precedence
over the handoff. A child agent gets its own executor/host identity, and retains
the inherited work root and immediate parent. Separate concurrent host sessions
in the same checkout derive separate roots.

## Caller event adapter

`nexus work host-event --host codex FILE` accepts a bounded native hook object;
`-` reads stdin. `--advisory` is reserved for installed hooks: no decision output,
no workload policy, and failures do not block the host. Explicit ingestion returns
a capture receipt and optional health. `durable`, `pending`, `rejected`, and
`lost` are distinct. A pending receipt does not promise persistence or server
acceptance; health can be temporarily unavailable while the writer owns the
outbox. The native command waits at most 1.5 seconds for local durability. Replay retained observations through `work record`, rather than
rerunning the workload.
Short-lived `nexus exec` and commit-review producers also allow up to 1.5 seconds
for their already admitted local capture jobs before the final delivery pass.
This wait occurs after measured execution finishes, so it does not extend the
recorded task duration. A deadline still leaves persistence and delivery unknown.

Malformed or unreadable host callbacks increment the bounded transport rejection
health counter without inventing a session or storing the payload/error text.
Installed advisory hooks still exit successfully and emit no host decision.
Inspect `transport_rejected` and `transport_rejection_reasons.invalid_host_event`
when a capture period has missing source boundaries.

Generic runner events use `ExecutionQueued`, `ExecutionStart`, `ExecutionPoll`,
and `ExecutionFinish`. Supply `session_id`, `work_session_id`, `execution_id`,
`parent_activity_id`, `executor`, `kind`, and `name` consistently. `occurred_at`
retains an authoritative source timestamp. Boundaries use the execution identity
and phase; `source_event_id` provides an authoritative title revision when a
caller has one. Finish supplies `outcome` and optional exact `elapsed`. For exact
source-event IDs and full retransmission guarantees, persist canonical work
observations and use `work record`.
A poll does not create or start an execution. A real retry needs a new
`execution_id`; link it with the existing `retry_of` work relationship.

Persist complete source envelopes before delivery. Repeat delivery keeps source
event IDs and times. Cancellation is a finish only when the execution owner
reports it; a cancel request or host stop is insufficient. A queue rejection can
finish without a start, preserving that missing boundary. Distinguish a canceled
attempt, timeout, infrastructure failure, ordinary failure, and unknown outcome.

Native callbacks stamp UTC occurrence plus an OS-owned monotonic source clock
before queueing or optional title lookup. On a matching OS boot clock, retained
start/finish ticks provide `os_monotonic_callback_interval`, even across command
hook processes. Each producer retains its own process identity. This measures
observed callback boundaries and excludes time the monotonic source does not
advance, including suspend on affected systems; it is not CPU time or an exact
measure of internal model reasoning. Missing/mismatched clocks remain unknown.
An explicitly supplied historical `occurred_at` disables generated current
source-clock ticks. Runners should retain their own elapsed measurements.

`session_title` explicitly supplies authoritative host title metadata. Native
callbacks can resolve Codex's exact thread name through read-only `thread/read`,
or Claude's title metadata in the supplied, session-matched transcript. Lookup
is bounded and optional. Title updates rename the same host activity and mirror
to an automatically owned root; they preserve caller-owned shared work titles.
Absent authoritative metadata, the source host ID remains the visible fallback.
No title is synthesized from prompt text.

## Verify a coverage period

Use an isolated project and configured outbox. Start two human requests in one
host session; finish one normally and interrupt or fail the other using a
supported source. Launch a child and a measured command, then an asynchronous
execution with repeated polls that outlives dispatch. Query the same root and
confirm distinct identities, starts, terminal outcomes, and unchanged workload
exit/output. Repeat after companion restart. Check `work health` and explicit
recording receipts. Keep missing callbacks and unsupported sources visible.
Provider-backed verification is opt-in and is not part of ordinary CI.

Live Codex CLI 0.165.0 verification found that native Bash hook payloads contain
`command` but omit the tool's execution `workdir`; the callback `cwd` remains the
host startup directory. Therefore cross-checkout Git operations must carry their
target explicitly, for example `git -C /path/to/checkout merge --no-commit topic`,
even when the native tool also sets `workdir`. Keep integrations in separate
native calls. The installed prompt hook supplies this guidance. Nexus cannot
recover a directory the host never reports, and does not guess from another
agent's checkout or nearby transcript entries. Live verification covered warning
delivery before an actual cross-checkout Git merge, completed request timing, and
native session-end claim cleanup. Treat this as version-specific evidence, not a
promise about all host versions or omitted callback fields.
