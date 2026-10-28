# Integrating caller-owned work with Nexus

## Locate the installed interface

`nexus setup` installs this skill for Codex and Claude, including two canonical
helpers under the installed skill's `scripts/` directory:

- `work-capture.sh`: sourceable Bash functions for fail-open named intervals and
  Git-hook observations. Source it; it need not be executable.
- `nexus-work-github.py`: a local snapshot-to-observation adapter invoked with
  Python 3.11 or newer. It does not contact GitHub or execute CI.

Resolve the skill directory from the loaded skill location. Do not assume the
caller repository contains Nexus's scripts or hardcode a user's home. In a
Nexus source checkout, canonical helpers live in the repository's top-level
`scripts/`; setup bundles those same sources beside the installed skill. The
Markdown source remains under `skills/nexus-work-history/`.

Inspect `nexus exec --help`, `nexus work --help`, and `nexus work schema` before
implementing an adapter. `nexus doctor` checks configured service reachability.
Existing host hooks capture supported prompt/tool/session callbacks; extend
uncovered boundaries instead of duplicating those observations.

Choose the same Nexus project binding for repositories participating in one
tree. For remote service access, preserve the configured endpoint/project and
use the deployment's existing secret mechanism. Keep credentials out of records
and untrusted CI artifacts. When an explicit configuration is needed, propagate
an absolute `NEXUS_CONFIG` path or use `nexus --config /absolute/config.toml ...`.
The command wrapper passes an explicit selected configuration to descendants.

## One root, distinct executions

These IDs and command paths are examples. Resolve an existing logical root or
create one once, using a unique ID or retaining the returned `subject.id`.
Repeated `work open` calls are not a way to reconnect to an existing root.

```sh
nexus work open "Improve router fallback" --id router-change-42
export NEXUS_WORK_SESSION_ID=router-change-42
# Preserve the host's real identity when available.
export NEXUS_HOST_SESSION_ID=host-session-17
export NEXUS_AGENT=implementation-agent
nexus work start "Implementation" --id implementation-42 \
  --kind agent.task --work-session "$NEXUS_WORK_SESSION_ID"
export NEXUS_PARENT_ACTIVITY_ID=implementation-42
```

Carry `NEXUS_WORK_SESSION_ID` and `NEXUS_PARENT_ACTIVITY_ID` across process,
subagent, and machine boundaries explicitly. Preserve `NEXUS_HOST_SESSION_ID`,
`NEXUS_AGENT`, `NEXUS_MODEL`, and `NEXUS_TURN_ID` where known. A real child agent
gets its own host/agent/model identity while retaining the shared logical root
and immediate parent. Do not attribute its execution to the dispatcher.

`exec` creates a fresh activity, measures the synchronous child command, and
passes that activity's ID as the parent for descendants:

```sh
nexus exec --kind test --name "Router tests" -- ./scripts/test-router.sh
NEXUS_WORK_ENVIRONMENT=beta nexus exec --kind evaluation \
  --name "Beta evaluation" -- ./scripts/evaluate-beta.sh
```

The caller chooses these commands, environments, and criteria. Wrapping a
dispatch command measures dispatch unless it also waits for actual completion.
Use lifecycle observations for an asynchronous child's real lifetime.

A caller-managed retry gets a new activity with a link to the earlier attempt:

```sh
nexus work start "Router tests, retry" --id test-attempt-2 --kind test \
  --parent implementation-42 --retry-of test-attempt-1
```

`start` declares and starts the attempt; it does not run tests. Record its actual
completion separately. `finish` defaults to activity scope; root completion
needs `--scope session`. A measured finish can supply `--elapsed-ns`,
`--timing-method`, and `--clock-id`. Use exact decimal text from the owning
process's measurement. Separate CLI processes have different producer instances
and do not promise a derived duration. Prefer `exec` for synchronous work.

## Wire scripts and Git hooks without changing their policy

Set `nexus_skill_dir` to the loaded installed skill directory. Reuse the helper,
or vendor its canonical source into the caller repository when the script must
run without that installation. The shell helper has no dependency on Markdown
or the GitHub adapter.

```sh
source "$nexus_skill_dir/scripts/work-capture.sh"
nexus_work_exec test "Router tests" ./scripts/test-router.sh
```

`nexus_work_exec KIND NAME COMMAND [ARG ...]` selects a compatible, configured
Nexus binary when available and otherwise runs the original workload directly.
`NEXUS_WORK_BINARY` can select an absolute executable. The helper retains child
output and exit result and never retries the workload. Preserve existing script
error handling: a real test failure stays a failure. Never use
`nexus exec ... || original-command`; the first call may already have executed
and failed, so that pattern repeats it.

Wrap the enclosing commit to supply exact correlation:

```sh
nexus_work_exec commit.attempt "Commit router change" git commit -m "Fix router fallback"
```

That wrapper exports `NEXUS_COMMIT_OPERATION_ID`. Existing pre/post-commit hooks
can source the same helper and add advisory observations around their existing
work, preserving their checks and exit policy:

```sh
# pre-commit, before existing validation:
nexus_work_git_hook pre-commit
# post-commit, after the commit exists:
nexus_work_git_hook post-commit
```

Guard sourcing optional resources when a host must run without them: the helper
handles an unavailable Nexus executable, but cannot be sourced from a missing
file. Telemetry must not turn hook success into failure. Nexus's own
`scripts/install-hooks.sh` enables this checkout's tracked hooks; it is not a
generic installer for another repository.

A bare commit lacks the enclosing operation ID. Retain observed hook intervals
without guessing commit association from matching trees, timestamps, or process
IDs. Likewise, stopping an agent or process does not finish its descendants or
the logical root.

## Record source events through CLI or MCP

Convenience `open/start/finish/milestone/link` commands create new observation IDs
on every call. Use them for one-time facts. For delivery surviving retries,
persist complete observations before sending, retain event IDs/times/producer
identity, and replay the same file. A new execution needs a new activity ID; a
retransmission does not.

`nexus work record FILE` accepts this envelope; `-` reads stdin:

```json
{
  "format_version": 1,
  "observations": [{
    "id": "runner-17:test-1:declared",
    "subject": {"scope": "activity", "id": "runner-17:test-1"},
    "occurred_at": "2026-01-02T03:04:05Z",
    "producer": {"name": "example-runner", "version": "1", "instance_id": "runner-17"},
    "method": "source_hook",
    "body": {
      "event": "activity_declared", "name": "Router tests", "kind": "test",
      "work_session_id": "router-change-42", "parent_activity_id": "implementation-42",
      "executor": "runner-17:worker-4",
      "context": {"environment": "ci", "labels": {"nexus.accounting.role": "execution", "purpose": "Verify fallback after the motivating routing failure"}}
    }
  }]
}
```

Replace IDs, time, and purpose with execution facts. Add separate `started` and
`finished` observations with distinct event IDs for the same subject; `queued`
is different from `started`. A finish retains outcome, optional exact elapsed
duration, exit code/signal, and assessment. Use the live schema for these fields,
typed links, and corrections. Occurrence time describes the source event, not
the later import. A new producer process needs a distinct clock identity; a
stable provider execution identity can describe timestamps from that provider.
Keep unknown boundaries unknown. Use `context.labels["nexus.accounting.role"]`
with `container`, `execution`, or `wait` only when the source establishes that
meaning. Containers describe enclosing lifetimes; they do not explain all time
inside them. Unknown roles remain unattributed. An `executor` identifies a real
worker instance; provider/tool names and runner labels are not worker identities.

Each atomic request contains at most 64 observations, each at most 16 KiB;
CLI files have a 768 KiB limit. Direct recording requires acknowledgment.
Invalid input or immutable-content conflicts require repair rather than endless
retries. Persist transient delivery failures without repeating the work.
Nexus binds server receipts and authenticated submitter provenance; do not forge
them. Use import/export to preserve reported original source receipts.

The `nexus_work` MCP tool takes a direct `WorkAction`. To record, send
`action: "record"` and the envelope's observation array, omitting the CLI
`format_version`. A query example is:

```json
{"action":"timeline","work_session_id":"router-change-42","parent_activity_id":"implementation-42","limit":24}
```

Shared CI keeps one canonical activity, with additional session memberships:

```sh
nexus work link ci-run-17 --relation member_of \
  --target '{"kind":"session","id":"another-work-session"}'
nexus work link test-attempt-2 --relation evidence \
  --target '{"kind":"eval","id":"recorded-eval-id"}'
```

## Import provider history and recover delivery

A GitHub snapshot contains `repository`, `observed_at`, `workflow_run`, and
`jobs`. The workflow identifies one exact `run_attempt`; jobs and steps belong
to it. Optional `pull_request` holds its provider object. The installed adapter
transforms local metadata without network access or service credentials:

```sh
python3 "$nexus_skill_dir/scripts/nexus-work-github.py" run-snapshot.json \
  --work-session router-change-42 --parent implementation-42 \
  --output-directory nexus-observations
(
  set -e
  for page in nexus-observations/*.json; do nexus work record "$page"; done
)
```

Use a new or empty output directory and ingest every page. Large snapshots need
multiple bounded files; stdout mode fails rather than dropping excess records.
`--parent` attaches the canonical workflow execution to the initiating request or
activity through relationship evidence. It does not redeclare or duplicate a
provider run already imported under another work root. Carry the actual parent
from the [host identity handoff](host-adapters.md), not a nearby session.
The adapter preserves provider run/attempt/job/step identities and keeps mutable
metadata separate. `updated_at` is not an observed completion time, and a workflow
head SHA does not prove what a job actually checked out.

Workflow and job lifetimes are accounting containers; observed steps are
execution intervals. The adapter uses a positive integer job `runner_id` from
[GitHub job metadata](https://docs.github.com/en/rest/actions/workflow-jobs) as
`github:REPOSITORY:runner:ID` for that job and its steps. A missing, unassigned,
or invalid runner ID keeps worker occupancy unknown. It never substitutes a
runner name, provider name, checkout path, or neighboring job's identity.

Accounting evidence is additive. `Accounting role observed` milestones carry
`nexus.accounting.role`; `Executor identity observed` milestones carry
`nexus.executor.declaration: "unqualified"` and an optional `nexus.executor.id`.
These mark the adapter's historical `github-actions` declaration value as a
provider label, preserving existing declarations and event identities. An
assigned worker arriving later does not redeclare accepted work. Missing later
metadata does not erase a known worker; competing positive identities remain
explicit conflicts with unknown worker occupancy. The same generic milestone
contract lets other caller-owned adapters qualify old categorical executor labels.

Automatic host/wrapper capture uses a private, bounded local outbox. `work health`
reports queued, retained rejected, recent delivery, bytes, known losses, and
`rejected_evicted`. `work flush` attempts bounded delivery. Active clients retry;
once they stop, delivery waits for another client or explicit flush. Queued,
rejected, and acknowledged are different states. `work export FILE` / `import
FILE` exchange checksummed work-only JSONL, distinct from database-wide transfer.
`exec`, `review-commit`, and `work git-hook` use a bounded local capture drain before their final
flush and exit, after workload measurement. A pending receipt or a timed-out
drain still does not establish durable capture or server acknowledgment.
Call `work git-hook` synchronously and keep hook failures advisory. Do not
background the command: its process must remain alive to finish admitted capture.

## Verify the resulting view

Use `work list` for roots, `work activities --kind test --outcome failed` for
attempts, `work timeline ROOT --parent ID` for immediate children, `work show ID`
for detail, and `work events ID` for immutable observations. Inspect roots with
`--scope session`. Follow `next_cursor` with `--after`; ordinary projections
can reflect later corrections even though their subject population has a
receipt cutoff. Preserve truncation/conflict/gap notices.

Exercise descendants, repeated tests, shared membership where relevant, and
unavailable capture. Verify workload output/status and exactly one execution
alongside delivery. The Work tab in native `nexus tui` and Pi `/nexus` defaults to session
wall-duration summaries, activity-kind duration bars, and daily history charts.
`m` switches means/totals; `t` switches session/activity trends; `[` / `]`
selects an activity-kind series. Enter on a parent session opens a visual
breakdown of retained descendants without requiring manual tree expansion.
Time runs downward on a shared UTC source-boundary axis; child tasks branch
right at source admission/start, with separate lanes for overlapping work.
Use arrows/PageUp/PageDown to pan time, `+`/`-` to zoom, left/right to pan lanes,
and `j`/`k` then Enter to inspect a task. `g` opens this visual for a session
reached through the paginated record browser. Queued tasks retain a separate
queue-wait segment on the same lane; queue sums can overlap and never count as
worker occupancy. Inclusive intervals overlap and are not additive effort. Missing or conflicted
elapsed measurements remain unknown and are excluded from means; displayed
sample counts explain that denominator. Daily buckets use UTC start, falling
back to the subject's first source occurrence if no start was captured.

The dashboard uses a bounded server projection of recent retained subjects,
not the first page of the record browser. Source-population, byte, chart-row,
and selectable-session caps produce explicit partial-history notices. These
limits and unsupported child internals prevent a completeness claim. Press `d`
for the secondary record browser, filters, expandable hierarchy, exact elapsed
text, links, and immutable observations. The MCP read shape is
`{"action":"dashboard"}` or
`{"action":"dashboard","work_session_id":"router-change-42"}` on `nexus_work`;
it never executes or retries the caller's work.
