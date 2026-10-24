---
name: nexus
description: Choose Nexus capabilities and connect an existing agent or repository workflow to them. Use when adopting Nexus, finding feature-specific guidance, or deciding which records and integrations support current work.
---

# Fit Nexus to the work

Nexus gives agents shared coordination, durable context, and an attributable
history of what happened and why. Connect these capabilities to the caller's
existing repository, agents, tests, eval runners, and delivery process. Nexus
does not become the owner of that process or select its goals.

Prefer the installed host hooks for ordinary coordination and usage capture.
Add explicit records where they carry information that cannot be observed:
motivation, durable knowledge, evidence, child identity, or a missing lifecycle
boundary. Do not require a new manual ritual before every edit or tool call.

Read the skill matching the task below. Load further skills only when the work
crosses their boundaries. The installed CLI's `--help`, domain schemas where
provided, and MCP tool input schemas define the available interface. A Nexus
source checkout is required only for source-development commands.

## Select a capability

| Feature | How it relates to existing work | Skill |
| --- | --- | --- |
| `installation` | Install the binary, bundled skills, host integrations, and repository binding. | [nexus-setup](../nexus-setup/SKILL.md) |
| `configuration` | Keep hosts and worktrees on the intended local or remote project and configuration. | [nexus-setup](../nexus-setup/SKILL.md) |
| `runtime` | Integrate through CLI, MCP, and existing lifecycle hooks using the selected transport. | [nexus-setup](../nexus-setup/SKILL.md) |
| `coordination` | Observe active sessions, claims, overlaps, and Git reconciliation while editing. | [nexus-coordination](../nexus-coordination/SKILL.md) |
| `analyst` | Request optional model analysis when an actual coordination conflict needs interpretation. | [nexus-coordination](../nexus-coordination/SKILL.md) |
| `agent-memory` | Retain bounded, deliberate notes and retrieve relevant knowledge across sessions. | [nexus-agent-memory](../nexus-agent-memory/SKILL.md) |
| `papercuts` | Record recurring development friction, support existing reports, and retain resolution history. | [nexus-papercuts](../nexus-papercuts/SKILL.md) |
| `sos` | Record an immediate agent blocker and preserve the evidence and triage history. | [nexus-sos](../nexus-sos/SKILL.md) |
| `usage-analytics` | Understand available versus used tools, skills, scripts, and semantic activity categories. | [nexus-usage-analytics](../nexus-usage-analytics/SKILL.md) |
| `observability-ui` | Inspect current coordination and historical records through the native or Pi terminal interface. | [nexus-observability](../nexus-observability/SKILL.md) |
| `work-history` | Preserve logical work, nested attempts, agents, timing, CI, and delivery evidence across sessions. | [nexus-work-history](../nexus-work-history/SKILL.md) |
| `evals` | Retain eval versions, reasons for change, call context, flexible metrics, artifacts, and caller-run results. | [nexus-eval-history](../nexus-eval-history/SKILL.md) |
| `architecture-tooling` | Find and maintain feature-shaped source ownership and specifications when changing Nexus. | [nexus-development](../nexus-development/SKILL.md) |
| `complexity-analysis` | Use structural measurements and repetition evidence to review Nexus changes. | [nexus-development](../nexus-development/SKILL.md) |
| `commit-review` | Combine deterministic validation with advisory architecture and deletion reviews. | [nexus-development](../nexus-development/SKILL.md) |
| `persistence` | Extend durable feature state, typed Diesel access, migrations, and transfer semantics in Nexus. | [nexus-development](../nexus-development/SKILL.md) |

For eval work, use [nexus-build-eval](../nexus-build-eval/SKILL.md) to construct
an evaluation, [nexus-eval-audit](../nexus-eval-audit/SKILL.md) to assess its
validity and evidence, or [nexus-hillclimb](../nexus-hillclimb/SKILL.md) to guide
caller-run improvement. Each uses the same historical evidence contract.

## Connect capabilities without collapsing their meaning

An existing coding flow can use coordination while editing, memory for a
decision worth retaining, work history for the implementation and its child
reviews/tests, and eval history for the versioned measurement and result. Usage
analytics answers which capabilities agents used and what availability was
observed. Invocation counts alone do not establish availability or task
duration. Papercuts track friction;
SOS records an immediate blocker. Link related evidence where the API supports
it, retaining each record's purpose and source identity.

Bind participants to the intended Nexus project before correlating their
records. A host session is a participant; a logical work session can span
multiple hosts, prompts, PRs, and environments. Preserve real child identities
and known links rather than inferring completion or causality from proximity.

Automatic usage and work-history telemetry is metadata-only and fail-open.
Coordination hooks and deliberate memory, issue, and eval submissions have their
own content and privacy contracts; do not silently turn analytics into
prompt/output collection. Report recording failures separately
from workload outcomes. Nexus advisories and model reviews inform decisions;
the caller retains execution policy, success criteria, retry policy, and
deployment authority.

Inspect the resulting records with the relevant CLI/MCP reads or terminal
views. Hand over project and record identifiers, integration locations,
verified coverage, and known gaps so the next agent can continue from evidence.
