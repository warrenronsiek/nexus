---
name: nexus-usage-analytics
description: Integrate or interpret Nexus tool, script, skill, activity, and availability analytics, including unused supplied tools and recurring work without an official capability.
---

# Understand how agents interface with the world

Nexus records concrete capabilities and the activities they serve. The intent is
to answer what agents do, whether they use resources supplied to them, and which
repeated work suggests a missing tool or script. Keep the host tool identity,
named repository script, or standard CLI visible alongside inferred purpose.
`bash`, `python`, and a generic `script` bucket alone cannot answer those questions.

The analytics are observational and advisory. They do not enforce tool choice,
claim complete host visibility, or prove that inferred program branches executed.
The caller owns its workflow and decisions about adopting or creating tools.

## Use the existing capture first

`nexus setup` wires Codex and Claude lifecycle hooks. Pi observes tools and
explicit `/skill:name` activation whenever its installed extension is loaded,
independently of whether `/nexus` is open. The stdio companion performs bounded
checkout-side script/skill detection and semantic enrichment in local or remote
mode; the shared server never opens client paths or executes the classifier.

Keep reusable named scripts at their natural repository-relative paths. Nexus
conservatively resolves supported shell invocations and inventories `scripts/`,
`tools/`, and `.githooks/` before execution. It distinguishes preexisting scripts
from ones introduced during a session only when coverage supports that claim.
An arbitrary path, incomplete snapshot, or opaque command can remain unknown.
Do not rename actual host tools or move unrelated files merely to improve a chart.

For an existing script automatic inference cannot identify, the installed
fallback preserves stdio and exit behavior:

```sh
nexus exec -- python3 ./scripts/check_data.py
```

This observation appears in **Legacy**; it does not provide the entire semantic
analytics or availability contract. For durable task timing and causal lineage,
use [work history](../nexus-work-history/SKILL.md) and its documented wrapper
options. Avoid wrapping the
same command repeatedly just to increase observations.

## Record availability separately from use

A zero-use tool is informative only when there is evidence it was offered.
Pi reports configured/active tool definitions and fingerprints. Codex/Claude
observed tool calls do not imply a complete host-wide inventory. Preserve that
unknown rather than manufacturing a denominator.

Read [host-adapter integration](references/integration.md) when implementing
custom lifecycle/inventory capture. Inventory resources with actual names,
origins, exposure, safe descriptions, and versions; update when definitions or
active tools change. Keep `declared`, `discoverable`, and `exposed` evidence
distinct. Session exposure means available at some observed point, not proven
available at every call. A project namespace is not an access-control boundary.

## Classify purpose without replacing facts

Deterministic rules recognize known CLI operations and named scripts first.
Opaque inline work can receive a bounded generic purpose and existing or new
activity category from the configured classifier. Facts remain independent of
category revisions; failed or unavailable inference stays unclassified.

Classification runs asynchronously using the analyst provider/model even when
conflict analysis is disabled by `analyst.enabled`. `[analytics]` controls
`classification_enabled`, `classification_timeout_seconds` (default 30), and
`max_classifications_per_day` (default 128). Matching patterns reuse labels;
classification is not a required extra model call for every invocation.

Analytics persistence excludes raw commands, arguments, environment values,
source, output, error text, and classifier prompts. Enrichment temporarily uses
bounded, locally redacted input and may send it to the configured provider;
redaction is best effort and custom identifiers can remain. Disable semantic
classification when that provider boundary is unsuitable. Keep inventory
descriptions and persisted category/purpose labels generic and free of secrets.

## Inspect evidence and counting semantics

Open `nexus tui` or Pi `/nexus`. Tools provides **Activities**, **Capabilities**,
**Opportunities**, and **Legacy**; Skills has its own tab. All use rolling
seven-day observations. For agents, discover and call `nexus_analytics`:

```json
{"action":"summary"}
{"action":"catalog","offset":0,"limit":32}
{"action":"opportunities"}
{"action":"categories"}
```

Pass `project_id` when explicitly inspecting a project; an omitted scope follows
the configured transport's read binding. Page catalog and implementation examples
instead of treating the first page as a complete registry. `nexus_usage` exposes
legacy tool/script/skill counts and capture health. There is no separate
`nexus analytics` CLI command; use MCP or the terminal.

Report invocation counts separately from identified operations: one call may
contain multiple operations/categories, and a parent with observed child calls
does not contribute duplicate activity operations. Supplied and improvised
implementations in the same category are leads for review, not proof that one
could replace another. Missing completion is not success, unknown availability
is not zero use, and historical private arguments cannot be reconstructed.

Inspect provenance, classification model/policy, coverage, truncation, and capture
health before drawing conclusions. A useful handoff identifies concrete repeated
work, the observed supplied alternatives, and what evidence is missing. Verify
both a named script and an inline operation, inventory with a genuinely unused
tool, and unavailable capture that preserves the workload.
