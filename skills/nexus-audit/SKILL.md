---
name: nexus-audit
description: Register project controls explicitly, submit immutable run and follow-up records, and export evidence for an auditor. Use when an agent operates a defined review, approval, or other internal control.
---

# Record internal control evidence

Use `nexus audit schema` or the input schema of `nexus_audit` for the typed
contract. Bind the caller to the intended Nexus project. Use an explicit
`append` action for every definition or observation. Save the input and reuse
its ID when retrying delivery. An identical replay returns the original receipt.

## Carry evidence through a task

When a repository requires control evidence for reviews, validation or delivery,
read [task evidence](references/task-evidence.md). Identify applicable procedures
at task start, reuse dedicated recorders, preserve failed attempts and delivery
retries, and reconcile exact subjects against acknowledged run IDs before
completion. The bundled read-only checker exposes missing or mismatched evidence;
it does not supply release authority or discover an omitted population.

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

## Read recorded executions and attach files

Use `nexus audit list --control KEY --version VERSION` to find matching runs.
The optional RFC 3339 `--from` and `--until` bound occurrence time, with the
start included and the end excluded. Use `nexus audit read request.json` or
`nexus_audit` for a pinned control definition, journal, or additional pages. For
paged reads reuse the returned receipt cutoff with the next `after` cursor.

Use `nexus eval upload resolution.md` for an approved verification report or
other file. It stores the bytes with a SHA-256 digest in the current project.
Use the returned digest in the evidence list of the run, finding, or follow-up.

```json
{ "kind": "stored_blob", "digest": "<SHA256>", "description": "resolution.md" }
```

Use `nexus eval download <SHA256> resolution.md` to retrieve the verified bytes.
An exported blob uses the digest as its filename. Keep a useful description
with the evidence reference.

## Hand over the evidence

```sh
nexus audit export review-evidence --from 2026-11-01T00:00:00Z --until 2026-12-01T00:00:00Z
nexus audit export review-sample --run RUN_ID --include-blob <SHA256>
nexus audit verify-export review-evidence
```

Share the manifest and package with the eligible population supplied separately.
The default export includes journal records and references. Use
`--include-blob SHA256` for approved verified contents or `--redact-evidence`
to replace references with redaction markers. Redaction does not change
narrative fields. Examine the requested disclosure before sharing. In the
authenticated Audits view, use the rolling range or explicit UTC dates, add a
control or version filter, and select a chart bar or run to inspect recorded
executions and follow-up. When both date fields are blank, the range sets the
lower bound. The presets are 24 hours, 7 days, 14 days, and 30 days, with seven
days as the initial selection.
