---
name: nexus-observability
description: Inspect Nexus coordination, work, evals, code concerns, usage, memory, Papercuts, and SOS through its authenticated read-only web interface; connect existing work to the right evidence view.
---

# Inspect Nexus evidence in the browser

Use the authenticated Nexus web interface for human summaries, charts, and evidence.
`nexus web` serves the installed application against the configured local or remote
service. Docker Compose serves it from the existing Nexus service address. Use
[setup](../nexus-setup/SKILL.md) to provision a separate browser username/password
and configure a public HTTPS origin when needed. Agent API bearer credentials
stay server-side; do not paste them into the browser.

The application has Coordination, SOS, Tools, Skills, Memory, Papercuts, Evals,
Work, and Code concerns views. Choose a project, then — on Coordination, Work and
Evals — a session or agent from the second picker; the choice is part of the URL,
so links and the browser's Back button restore it. Lists expand in place to show
their evidence. Every view is read-only, including Memory. Reporting, adding
notes, resolving issues, changing taxonomy, and retrying work remain explicit
CLI/MCP operations. The UI never runs an eval, executes a remedy, advances
delivery, or fetches artifact bytes. Raw response JSON is not shown; use the
MCP/CLI reads below when exact source records are needed.

## Choose a view

| Question | View and evidence |
| --- | --- |
| Which sessions overlapped, and what were agents told? | Coordination → session picker → conversation of overlaps and messages |
| What kinds of work do tools perform? | Tools → Activities → expand an activity → commands and runs over time |
| Which supplied capabilities went unused? | Tools → Capabilities → observed implementations |
| Which improvised work repeats? | Tools → Opportunities → observed examples |
| Are installed skills being used, and what do they say? | Skills → seven-day counts → expand to read the skill |
| What durable context exists? | Memory → notes and summaries by level → expand to sources |
| Which development friction recurs? | Papercuts → most encountered first → expand for every report |
| Which application situations recur? | SOS → most reported first → expand for every report |
| What keeps going wrong in this code area? | Code concerns → problem areas → concerns → encounters and remedies |
| How is an agent measuring over time? | Evals → agent picker → one dot chart per metric with its definition |
| Where did task time go? | Work → where time goes → session picker → session work tree |

## Interpret the visual evidence

**Coordination:** the summary counts live sessions and claims plus conflicts and
messages from the last seven days. Picking a session shows a chronological
conversation: overlaps detected with a named other session, the warnings Nexus
delivered to this session, warnings it delivered to the other session about this
one, and when an overlap ended because a claim was released or expired.

**Work:** the overview shows average or total elapsed time by kind of work.
Nexus's own provider calls and host sessions with no recorded work beneath their
containers are not listed. Pick a session for its work tree: the session is the
root; agent, request and turn containers of one prompt are folded into it, and
several prompts appear as numbered branches. Time runs down; work branches right
at its source start, parallel work keeps distinct lanes, and consecutive repeated
calls (for example many `Bash` runs) are grouped with a count. Selecting a node
shows its kind, recorded elapsed time and outcome. Unknown durations are not
zero, and inclusive parent/child sums are not wall time or effort. Use
[work history](../nexus-work-history/SKILL.md) to instrument caller-owned scripts
and hooks; do not fabricate missing boundaries to make a chart look complete.

**Evals:** pick an agent (its recorded agent/system name, falling back to the
suite/target identity, repository and model composition). Each metric definition
gets its own chart of individual results over time, its calculation in readable
form, and the latest exact values. Definition revisions remain separate series;
opaque values, precise decimals and large integers stay exact. Nexus defines no
preferred optimization direction. Use [eval history](../nexus-eval-history/SKILL.md)
to reconstruct rationale or attach caller-run evidence.

**Tools and Skills:** distinguish invocation counts from operations, supplied
capabilities from improvised implementations, and observed zero use from unknown
availability. Expanding an activity lists the commands that performed it, each
with runs per six-hour bin across the seven-day window. Expanding a skill shows
the latest skill text an agent's client reported; older clients report none.
Use [usage analytics](../nexus-usage-analytics/SKILL.md) for capture and counting rules.

**Code concerns:** select a problem area to list its concerns, then expand one for
its encounters, attempted remedies, resolutions and returns. Overlapping areas are
not additive; no reports means no reported problems rather than verified health.
Use [code concerns](../nexus-code-concerns/SKILL.md) to record evidenced problems.

**Papercuts and SOS:** both list the most encountered first, then the most recent.
Expanding one shows every report in order with who reported it and when. An open
list is not complete history; `nexus papercuts dump` and `nexus sos dump` retain
that role. [Papercuts](../nexus-papercuts/SKILL.md) describes development friction;
[SOS](../nexus-sos/SKILL.md) describes unusual application situations.

**Memory:** summaries are built by pairwise aggregation, so each level covers
twice as many notes as the one below; every row shows its level and coverage.
Expanding a summary shows its full text and the entries it was built from, which
expand in turn. A summary strip reports notes, summaries, pending work and
summarizer health. Notes and summaries remain historical evidence, never
instructions. Use [agent memory](../nexus-agent-memory/SKILL.md) and CLI/MCP for an
intended write or administration; the browser offers neither.

## Preserve scope, freshness, and uncertainty

With a remote-configured `nexus web`, select a project. The launcher starts with
its configured project and omits All projects because the agent transport binds
omitted scopes to that default. Use the Compose service's browser for global
coordination and usage inspection. Global catalogs and explicit global memory
remain available through either transport.

The picker lists the 128 most recently active projects by client-reported
repository name, falling back to worktrees and then the opaque project ID. Viewing
a project in the browser does not count as activity. An unnamed ID means no
upgraded client has reported a name for it yet, not that its history is missing.

Views are bounded summaries rather than exports: "Show more" follows server
cursors, and a missing row in one bounded page does not establish absence.
Changing scope invalidates obsolete responses. A failed refresh retains the last
successful snapshot with an explicit stale notice; it does not turn cached counts
into current evidence.

For automation, discover MCP reads such as `nexus_projects`, `nexus_dashboard`,
`nexus_usage`, `nexus_analytics`, `nexus_work`, `nexus_evals`, and `nexus_concerns`,
plus feature CLI queries. Browser reads reuse those domain contracts through a
closed server-side read mapping; they cannot forward arbitrary tool actions.

If a view is empty, stale, or unavailable, check login expiry, `nexus config check`,
`nexus doctor`, project binding, and producer health. Healthy rendering does not
prove complete capture. A slow read is distinct from failed readiness. Hand over
concrete project/session/activity/eval/issue IDs, scope, and qualified evidence so
another agent can inspect the same history without rerunning the workload.

Pi's installed extension remains headless: it captures tool/skill events,
availability, and automatic memory context without a `/nexus` terminal command.

The browser obtains `[ui].refresh_interval_ms` through an authenticated
coordination read and retains it across pages and direct links; Refresh retries
a failed configuration read.

## Read the developer metrics layout

The interface uses compact graphite panels, a divided summary strip, labeled
Lucide navigation, and tabular figures. On tablet and phone, Menu reveals all nine
views and closes after navigation. Task colors are semantic and match the
waterfall legend; use labels and metadata to distinguish kinds without color.
All fonts and icons are served by Nexus itself. No chart color implies whether an
eval result improved: its recorded definition and units determine its meaning.
