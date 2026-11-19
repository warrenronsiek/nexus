---
feature: audit-registry
---

# Audit registry

Nexus records project-scoped evidence of agent-operated controls. Agents explicitly
publish control versions, submit run results and findings, and append remediation
decisions and verification. Exported executions describe the observed cohort.
Management supplies the eligible population separately for reconciliation.

Each run pins its control version. Occurrence time is supplied by the caller and
receipt time is recorded by Nexus. Retries with the same ID and input retain the
first receipt and actor. The project journal advances monotonically, allowing
exports to freeze a receipt cutoff while selecting runs by occurrence time.

## Recording contract

`nexus audit schema` and the `nexus_audit` MCP tool describe the same
closed operations. The service binds writes to its project and actor context.
Control definitions own category codes, display labels and optional ordinal
scales. A run and its findings are one immutable append. Findings are
identified by run ID and finding ID. A retry is a new execution of the same
subject with an explicit link. A correction preserves the original subject,
control and finding IDs and publishes a new result against the previous
record. Control revisions, corrections and dispositions use expected
predecessor IDs. Verification links to the exact disposition that was checked.

Application writes use typed Diesel queries and serialize the append and its
predecessor check in one transaction on SQLite and PostgreSQL. Migration V28
and portable stream version nine retain audit records and receipt order.
Historic runs may report an obsolete or retired version to reflect the
procedure that actually ran. Controls and retirements remain in the journal
so a reader can compare the run with the documented effective dates.

The opt-in `review-commit --audit-control KEY@VERSION --audit-record PATH`
adapter records both perspectives in one execution. It captures the exact
patch digest supplied to both models and the observed or pinned version of the
base when available. Reviewers may also inspect the local checkout. The hash
identifies the supplied patch. It does not identify the additional files that
a reviewer read. The adapter saves a private project-bound input at a new path
before publishing. A malformed or failed perspective produces an inconclusive
result, while valid findings from the other perspective are retained.

## Reading and export

The authenticated **Audits** view provides a rolling range with 24-hour,
7-day, 14-day, and 30-day presets, plus UTC period, control, and version
filters. The rolling range supplies the occurrence-time lower bound when no
explicit start date is set and defaults to seven days. The total covers the
selected recorded cohort and the charts summarize
the displayed page. Chart selections identify the runs contributing to each
bar. Run details show the pinned definition, result and findings, receipt
metadata, and subsequent dispositions, corrections and verification. A run URL
preserves the selection, applied filters, and journal cutoff. Coverage
stays unknown because the eligible population is supplied separately.

`nexus audit export DIR` takes either a half-open RFC 3339 interval
`[--from, --until)` or explicit `--run ID` selections. The first read fixes the
project's journal cutoff. Each run's complete history through the cutoff
includes later follow-ups even when their occurrence is outside the run
interval. The JSONL contains immutable records. The CSV indexes contain scoped
execution and finding summaries, follow-ups, and chart data. The manifest
records selection and completeness counts, omitted or redacted references,
and SHA-256 hashes. The CLI can include explicitly selected, verified,
same-project Nexus blobs as `evidence/<sha256>`. It never dereferences external or Git links.
The offline verify command checks the package bytes against the manifest.

The deployment uses Nexus's shared credentials. Actor, executor, and verifier
provenance is reported in evidence, with enforcement of separation of duties
owned by the surrounding workflow. Repository and external evidence references
are caller assertions. A verified Nexus blob establishes retained byte identity.
Deployment backup and artifact retention determine how long the evidence
remains available. A manifest digest does not authenticate an actor or prevent
a database administrator from rewriting data.

See the [bundled audit skill](../../skills/nexus-audit/SKILL.md) for a sample
cross-model control and handoff workflow.
