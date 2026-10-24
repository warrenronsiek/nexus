# Record caller-owned delivery work

Build/install the current Nexus binary and run `nexus setup` to refresh the
existing host integration. `scripts/install-hooks.sh` enables this repository's
tracked Git hooks. Scripts can source `scripts/work-capture.sh` and use
`nexus_work_exec KIND NAME COMMAND [ARG ...]` for named boundaries. Set
`NEXUS_WORK_BINARY` to select a binary explicitly. The helper executes the original
command directly when a current capture-capable Nexus binary is unavailable.

Open one logical work session and propagate its returned `subject.id` through
`NEXUS_WORK_SESSION_ID`. Preserve the initiating host's identity when available:

```sh
nexus work open "Improve router fallback" --id router-fallback
export NEXUS_WORK_SESSION_ID=router-fallback
export NEXUS_HOST_SESSION_ID=my-host-session
export NEXUS_AGENT=my-agent
nexus exec --kind test --name "Router tests" -- cargo test --test router
nexus exec --name "Commit fallback" -- git commit -m "Improve router fallback"
```

The enclosing `git commit` wrapper supplies a commit-operation ID to both Git
hooks. Pre-commit records the candidate base/staged tree; post-commit records the
resulting full commit ID. An explicit `--kind commit.attempt` supplies the same
handoff for less conventional invocations. A bare Git command still has observed
pre-commit validation/review intervals when these hooks are enabled, but no
pre/post identity is guessed from matching trees, timestamps or process IDs.

Use `NEXUS_PARENT_ACTIVITY_ID` or `nexus exec --parent ID` for nested external
work. Pass the work/root/host context explicitly when dispatching another agent
or crossing a machine boundary. A spawn acknowledgment is not child completion.

The environment example runs any command supplied by the caller:

```sh
examples/work-history/delivery-step.sh dev deploy ./scripts/deploy-dev.sh
examples/work-history/delivery-step.sh dev evaluation ./scripts/evaluate-dev.sh
examples/work-history/delivery-step.sh production deploy ./scripts/deploy-prod.sh
examples/work-history/delivery-step.sh production verification ./scripts/verify-prod.sh
nexus work milestone router-fallback "Production verified" --environment production
```

Those sample workload paths belong to the caller. They are not Nexus commands
or required stages. The caller decides whether a verification establishes its
delivery milestone and can link its eval records separately. Commands preserve
their stdio and exit result; automatic metadata failures never rerun a workload.

For CI reconciliation, save a caller-collected snapshot with `repository`,
`observed_at`, `workflow_run`, and `jobs` fields. `workflow_run` is the provider's
run object for one exact `run_attempt`; `jobs` contains only that attempt's jobs
and their steps. Optional `pull_request` carries the provider's PR object.

```sh
python3 scripts/nexus-work-github.py run-snapshot.json \
  --work-session router-fallback > observations.json
nexus work record observations.json
```

The adapter reads local metadata and emits recording JSON. It performs no
network calls, runs no tests, and holds no service credentials. Callers can export
this metadata from restricted CI and ingest it later in a trusted process. For
larger snapshots, choose a new or empty output directory. The adapter writes every
observation in ordered files containing at most 64 observations and 768 KiB:

```sh
python3 scripts/nexus-work-github.py run-snapshot.json \
  --work-session router-fallback --output-directory nexus-observations
(
  set -e
  for page in nexus-observations/*.json; do
    nexus work record "$page"
  done
)
```

Default stdout mode reports an error when a snapshot requires multiple files.
The output directory must be empty so files from another snapshot cannot enter
the import accidentally. Replaying the same snapshot retains the same event IDs.

Source job/step start and completion times remain distinct from snapshot receipt.
A workflow's `updated_at` is not a completion boundary: absent an actual
`completed_at`, its conclusion becomes a status-observed milestone, and its end
remains missing. The adapter retains the provider-reported workflow head SHA,
run ID, attempt and job/step identities. The head SHA alone does not prove which
revision a job checked out. Canonical execution declarations and separate
membership links let one CI execution join several work sessions. Mutable names,
URLs, and statuses remain metadata observations; later availability does not
change the canonical execution or an existing completion report. Job/step source
boundaries allow Nexus to derive qualified wall-clock durations without treating
an updated timestamp as a finish.

Use `nexus work health` to inspect pending/rejected metadata and `nexus work flush`
to attempt bounded delivery. Active clients also retry. A stopped client cannot
promise delivery until another client or explicit flush runs. Direct `record`
requires a server acknowledgment; automatic adapters use the bounded local
outbox and disclose queuing separately.
