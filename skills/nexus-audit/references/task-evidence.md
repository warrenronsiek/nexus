# Reconcile a task's control evidence

Use this when a repository requires evidence for development, review, or delivery
work. The repository owns applicability and release policy. Work history captures
attempts and timing; Audit records execution of a defined procedure. Neither a
Work event nor a local file establishes an acknowledged Audit run.

At task start, read the repository's control procedure and resolve the applicable
control versions with `nexus audit read`. Record the expected population in a
private task plan: checks/reviews, exact subjects, outcome codes and any justified
exclusions. Include required deployment verification only when the user requested
that boundary. Reuse an existing definition for repeated executions. Define a new
procedure only with the owner's authorized scope; use its actual adoption date.
Never create a control per command, PR, or agent turn, or backdate a definition.

Record each actual execution with its full source revision or exact snapshot,
claimed executor, procedure/instruction revision, status, result, and safe source
references. Dedicated review/QA commands may already submit the run: reuse their
IDs instead of recording the same execution twice. Preserve failed attempts.
Keep raw logs, source content, prompts, screenshots and customer data in their
approved source systems. A test command's exit code proves only that check; a
model review remains advisory and supplies no human approval or deploy authority.

Save original project-bound append inputs privately. Retry the exact saved input
with `nexus audit append FILE`; never rerun a review, test, or deployment to fix
missing telemetry. Read the run back with its expected project before reporting
it as recorded. Link findings, dispositions and independent verification using
the run/finding IDs. A new revision or materially different scope invalidates an
old execution for the new subject. Do not rewrite the old run to match new work.

Before task completion, reconcile the declared population. The bundled checker
makes read-only calls to `nexus audit read` and pages through the server
run summaries at a pinned cutoff, selecting declared IDs in bounded batches.
It matches project, control version and exact subject. Nexus owns correction
and category projection. Use `nexus audit get` to inspect the original actor
and detailed follow-up when reconciling provenance or findings. Outcomes
requiring attention remain separate from receipt availability.

```sh
python3 PATH_TO_NEXUS_AUDIT_SKILL/scripts/nexus-audit-check.py /private/task-plan.json
```

A plan has this shape; replace every sample value with the actual scope:

```json
{
  "project_id": "actual-project-id",
  "expectations": [
    {
      "name": "Exact-commit security review",
      "control": {"key": "commit-security-review", "version": 1},
      "subject": {
        "kind": "git",
        "revision": {
                  "object_format": "sha1",
          "commit": "0123456789012345678901234567890123456789"
        }
      },
      "accepted_codes": ["pass"],
      "run_id": null
    },
    {"name": "Browser QA", "not_applicable": "Documentation-only change; no user-facing workflow affected"}
  ]
}
```

Fill `run_id` from the actual receipt, not a work activity or a queued event.
Use the adapter's exact staged-diff subject, including its base and patch digest,
when the review ran against staged work. External subjects carry the exact CI
attempt, deployment, or verified working-tree snapshot identity. Preserve the
underlying commit/digest in the external ID where the procedure requires it.
Do not substitute the resulting commit for an earlier staged subject.

Store the plan and any saved append inputs outside tracked source, preferably
under a task-specific directory in `git rev-parse --git-common-dir` (0700 directory,
0600 files). Export checker output to the same private location. Caller-supplied
IDs and labels can themselves be sensitive; inspect any report before sharing.
Repository integrations may vendor the checker byte-for-byte at a pinned Nexus
revision, so the workflow works before a CLI/skill upgrade.

Exit 0 means the declared entries have matching receipts with the explicitly
accepted categorical outcomes or reasoned exclusions. A run ID absent from a
successful pinned read is `unacknowledged`; a failed read is `unconfirmed`. Neither state proves a workload failure. Retry a saved
append only for the former after inspecting its source; restore reads for the
latter. Exit 1 exposes gaps or attention; exit 2 means the plan itself is invalid.
An empty population is invalid.
`not_applicable` is an explicit caller assertion, not an execution or approval.
The checker cannot discover omitted obligations or establish population coverage.
It does not override protected checks or repository-specific release dispositions.
For example, a permitted `limit_reached` outcome remains that outcome, never a
reviewer pass. An exclusion must never relabel a failed applicable execution.

Narrative controls and runs with findings need manual reconciliation of the full
history, dispositions and verification: the checker deliberately leaves these
requiring attention rather than guessing success. Keep that qualified result and
link the manual reconciliation in the source record. Report unresolved findings,
missing/unacknowledged/stale evidence, and actual authority separately. An outage
must not repeat or change the underlying workload. Follow the repository's
existing release policy and disclose evidence gaps in the completion report.
