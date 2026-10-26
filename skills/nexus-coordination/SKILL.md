---
name: nexus-coordination
description: Use Nexus to see concurrent agent work, interpret path-overlap advisories, and integrate lifecycle hooks. Use when agents share a repository, an overlap needs resolution, or a host needs coordination instrumentation.
---

# Make concurrent work visible

Nexus observes agent sessions and intended file changes so agents can detect
overlap before Git exposes a merge conflict. Claims are expiring signals of
activity. They grant no ownership or exclusivity. Severity comes from the
deterministic operation/path/hunk classifier; the caller decides how to proceed.
An advisory may justify splitting scope, sequencing work, accepting overlap, or
requesting review. It never becomes a Nexus permission gate.

## Fit Nexus around existing work

Prefer the installed host hooks over asking an agent to announce every edit.
Use [nexus-setup](../nexus-setup/SKILL.md) for `nexus setup`, repository onboarding,
configuration, and remote service wiring. `nexus integrate codex` or
`nexus integrate claude` prints the current registration and hook configuration.
Merge generated configuration with existing hooks; preserve unrelated behavior.

For a custom host, discover the installed MCP tools and their schemas. Bind a
stable host session ID and actual project root, then emit these boundaries:

| Boundary | MCP tool | Important identity |
| --- | --- | --- |
| User prompt | `nexus_user_prompt` | Session and prompt |
| Before an actual tool call | `nexus_pre_tool_use` | Session, unique `tool_use_id`, tool name/input |
| Successful completion | `nexus_post_tool_use` | Same session and `tool_use_id` |
| Failed completion | `nexus_post_tool_failure` | Same session and `tool_use_id` |
| Host session ends | `nexus_session_stop` | Same session |

Use genuine tool payloads where the adapter contract calls for them; unknown
tools do not gain claims by inventing a recognized name. The supported
extractors recognize file edits, patches and selected shell mutations. Git
reconciliation supplements hooks for active sessions, with bounded, file-wide
evidence. Neither path proves complete coverage of all edits.

Keep lifecycle delivery bounded and advisory. Malformed input, an unavailable
service, or failed persistence must not deny or repeat the underlying tool call.
Match completion to execution, and send session stop even when a task failed.
Do not equate a stopped host session with successful logical work; use
[nexus-work-history](../nexus-work-history/SKILL.md) for nested tasks, repeated
tests, cross-session lineage, and durations.

## Read and act on overlap

Run CLI commands from the relevant checkout with its normal Nexus configuration:

```sh
nexus sessions
nexus claims
nexus conflicts
nexus events --limit 32
```

Active lists and dashboard counters describe recent coordination evidence: sessions
age out after the claim TTL unless they retain a live lease, and open conflicts
require current claims from both original sessions on the path. Stop, release, or
expiry removes the live overlap without resolving or deleting its history. Use
`nexus sessions --all` and `nexus conflicts --all` for history. An
`advisory_ignored` event means successful continuation after an advisory; it does
not verify host delivery or agent acknowledgement.

The matching MCP tools are `nexus_sessions`, `nexus_claims`, `nexus_conflicts`,
and `nexus_events`. Read schemas for their optional project and scope filters;
CLI `--all` and MCP `scope: "all"` select historical states, but responses remain
bounded and expose `truncated`. An empty bounded view establishes only what that
view contains. Inspect actual files and Git state before making a merge decision.

When the caller has chosen a response, record it using the returned conflict ID:

```sh
nexus resolve "$CONFLICT_ID" --resolution 'Split the overlapping file by responsibility'
```

This changes the overlap's recorded state; it neither merges code nor releases
claims. A later stronger overlap may reopen it. When a session has genuinely
finished a scope, `nexus release "$SESSION_ID" --path src/example.rs` releases
that session's claims on the path; omitting `--path` releases its project claims.
Do not clear another active agent's claims merely to remove a warning. The
corresponding tools are `nexus_resolve` and `nexus_release`.

For a difficult stored overlap, `nexus analyze "$CONFLICT_ID"` or
`nexus_analyze` requests the configured read-only analyst. This is an explicit
provider invocation with normal configured cost and availability. It offers
advice, leaves deterministic severity intact, and does not edit or authorize
work. A failed analyst is an ordinary command error, not evidence of a code
defect. Do not enable or replace a user's provider merely to remove that error.

## Preserve identity and privacy

Local Git worktrees share project identity through the Git common directory;
independent clones need the same explicit remote project binding to share a
namespace. A project ID selects records and is not an authorization boundary.
Remote companions observe their own checkout; paths supplied by clients do not
give the service access to their files.

Keep agent, session, turn, invocation, and model labels faithful to the source.
Do not invent another agent's identity. Review the resolved privacy settings
before adding a new source: prompt synopses may be stored, while full prompts
are disabled by default. Coordination completion events can retain supplied
output/error payloads; minimize or sanitize those at a custom integration's
boundary. Keep credentials out of prompts and descriptive labels. Usage/work
telemetry has a separate, narrower metadata-only capture contract.

Verify a custom adapter with two isolated sessions that overlap a real fixture,
a failed tool call, and an unavailable Nexus service. Check that advisories
arrive, completion IDs match, session stop releases claims, and workload results
survive capture failure. Inspect the coordination panes with
[nexus-observability](../nexus-observability/SKILL.md). Use
[nexus](../nexus/SKILL.md) to choose another Nexus feature.
