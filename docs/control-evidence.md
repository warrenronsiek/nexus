# Development control evidence

At task start, load `nexus-audit` and its task-evidence reference. Inspect the
actual project with `nexus config show`, then read the applicable control
versions with `nexus audit read`. A review or validation is an execution of a
procedure; a command, PR or agent turn does not need its own control definition.

For Nexus development, Engineering owns these procedures:

| Control | Trigger and execution | Categorical results |
| --- | --- | --- |
| `cross-model-review@1` | Staged source change: run architecture and deletion via `nexus review-commit --audit-control cross-model-review@1 --audit-record PRIVATE_NEW_FILE` | `pass`, `changes_requested`, `inconclusive` |
| `repository-validation@1` | A development validation attempt: run the applicable focused checks or complete `./scripts/check.sh`, name the exact command and scope in safe evidence references, record exit status and instruction revision | `pass`, `fail`, `inconclusive` |

Register an absent definition explicitly using the Audit skill's schema and
actual adoption time. Reuse an existing version only when it describes this
procedure; a material procedure change requires the next immutable version and
its predecessor. The user's authorization to implement this workflow adopts
these Engineering procedures; it does not create an independent approval.
Model reviews remain advisory. Local deterministic checks and protected CI gates
retain their authority. Repository validation is `pass` only for the named
checks actually completed successfully; focused checks never imply the full gate.

The pre-commit hook saves a fresh private project-bound replay file under the
Git common directory and opts the existing cross-model command into Audit.
When Audit preflight is unavailable, the command still runs the ordinary review
once and reports a recording error. If delivery fails after a run, retry the
saved input with `nexus audit append FILE`; do not run the review again merely
to recover evidence. Retain the input and read back the exact run ID.
No private record or task plan belongs in a commit.

Record validation as an explicit `run` under `repository-validation@1`. Use the
full Git commit for a clean committed snapshot or an external source identity
with underlying commit and verified working-tree digest for uncommitted work.
A staged-diff subject covers only the staged patch: use it only when those exact
bytes were tested (for example, in an isolated index export), not when a normal
check read additional unstaged or untracked files. Include the actual executor,
elapsed time when measured, command/check scope, procedure revision, and retained
source references. Do not upload command arguments, logs, secrets or customer
content automatically. Preserve failed attempts and new IDs for real reruns.

Before reporting completion or merging, create/update a private task plan using
the Audit skill's task-evidence format and run:

```sh
python3 scripts/nexus-audit-check.py /private/task-plan.json
```

The plan names expected reviews and validation attempts with their exact subjects,
acceptable results and actual Audit run IDs. Include required CI or rollout
verification in the population when it belongs to the requested task. Reuse
native sources and existing runs; do not double-record their execution. Explain
inapplicable steps explicitly. Reconcile narrative outcomes, findings,
dispositions and verification manually, preserving the checker report and source
references. The checker conservatively marks findings as needing attention.

Report real test/review outcomes and missing, stale or unacknowledged evidence
separately. An Audit outage does not change or repeat the workload. Existing
release gates still apply. Work capture, a queued append or a model saying PASS
alone does not establish acknowledged control evidence. Human authorization and
independent verification remain in their authoritative sources.
