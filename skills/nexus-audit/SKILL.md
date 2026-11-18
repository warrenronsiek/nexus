---
name: nexus-audit
description: Register project controls explicitly, submit immutable run and follow-up records, and export evidence for an auditor. Use when an agent operates a defined review, approval, or other internal control.
---

# Record internal control evidence

Use `nexus audit schema` or the input schema of `nexus_audit` for the typed
contract. Bind the caller to the intended Nexus project. Use an explicit
`append` action for every definition or observation. Save the input and reuse
its ID when retrying delivery. An identical replay returns the original receipt.

## Define a control

Agree on the procedure and scope with the control owner. This is a sample
`AuditInput` for the existing cross-model commit review. Set the effective date
and occurrence time for the actual adoption and update the description to
match the agreed process.

```json
{
  "id": "cross-model-review-v1",
  "occurred_at": "2026-11-17T16:00:00Z",
  "event": {
    "kind": "define",
    "control": { "key": "cross-model-review", "version": 1 },
    "definition": {
      "owner": "Engineering",
      "objective": "Obtain a second model's assessment of agent-authored changes",
      "scope": "Agent-authored changes in this repository",
      "trigger": "Before a qualifying staged change is committed",
      "procedure": "Run the Nexus architecture and deletion passes, document exceptions, and link any subsequent disposition or verification.",
      "effective_at": "2026-11-17T16:00:00Z",
      "reason": "Initial registered procedure",
      "results": {
        "kind": "categorical",
        "codes": [
          { "code": "pass", "label": "Both review passes returned PASS" },
          { "code": "changes_requested", "label": "Changes requested" },
          { "code": "inconclusive", "label": "Review inconclusive" }
        ]
      }
    }
  }
}
```

Append the saved input with `nexus audit append control.json`. A revision adds
another `define` record with the next version and `previous` set to the
preceding control event ID. A `retire` references the current version and
definition ID and records the reason and effective date. The explicit version
on a run describes the procedure actually used, including late submissions.

## Run and follow up

```sh
nexus review-commit --audit-control cross-model-review@1 --audit-record review.json
nexus audit append review.json
```

The second command is a delivery retry. The private replay file pins the Nexus
project and the record ID. Ordinary hook invocations stay advisory. For other
workflows, append a `run` with an exact subject, pinned control version, claimed
executor, execution status, and result. Use an external source ID that can be
joined with the eligible population. For Git use a full object ID or a digest
of the exact staged diff. Attach stored blobs only after the Nexus blob API
has verified their SHA-256. Submit only content approved for the registry.

Read the run with `nexus audit get RUN_ID`. To document a fix, append a
`disposition` with that `run_id`, the `finding_id`, `disposition` set to
`remediated`, a reason, and a pinned fix revision or ticket. A first
disposition omits `previous`. A change or reopening takes the last disposition
ID. A later reviewer appends a `verification` referencing the disposition ID,
with their claimed identity, procedure, outcome, and supporting evidence.
Leave unknown follow-up visible. A correction uses `correct_run` with the
original run ID, latest run or correction ID as `previous`, a rationale, and
the replacement result. Keep existing finding IDs.

## Hand over the evidence

```sh
nexus audit export review-evidence --from 2026-11-01T00:00:00Z --until 2026-12-01T00:00:00Z
nexus audit export review-sample --run RUN_ID
nexus audit verify-export review-evidence
```

Share the manifest and package with the eligible population supplied separately.
The default export includes journal records and references. Use
`--include-blob SHA256` for approved verified contents or `--redact-evidence`
to replace references with redaction markers. Redaction does not change
narrative fields. Examine the requested disclosure before sharing. In the
authenticated Audits view, filter by control, version or UTC dates, select a
bar, and open the underlying execution and follow-up history.
