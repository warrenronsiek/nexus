---
feature: observability-ui
---

# Terminal observability UI

## What this feature does

Nexus has two entry points for one terminal interface: the bundled `nexus tui` command and `/nexus` inside Pi. Both render the same Coordination, SOS, Tools, Skills, Memory, Papercuts, Evals, Work, and Code concerns tabs. Coordination progressively discloses sessions, claims, conflicts, and events. SOS presents aggregate issues by importance and last arrival, then reveals their retained source reports, triage attempts, and lifecycle history. Papercuts lists the current or selected project’s open issues with encounter counts and latest encounter times; Enter opens the full description, identifiers, and timestamps. Pressing `n` or moving down from the last loaded row fetches older open reports in bounded pages. The list exposes whether more pages remain and points to `nexus papercuts dump` for the complete historical registry. Reads are abortable when leaving the tab, stale responses cannot replace a newer scope, and refresh retains the last successful list on failure. An initial timeout tells the user how to retry. Tools and Skills show rolling seven-day horizontal bar charts. Memory browses and searches notes, adds an explicitly scoped raw note, expands or invalidates a derived summary, and requests consolidation.

All data travels through one persistent local `nexus mcp` child per interface. Coordination, SOS, Papercuts, Code concerns, Evals, Work, project, usage, and memory reads use the same typed service boundary and configured client transport as the CLI. Memory requests carry correlated host context. Nexus serves an authenticated machine API and direct MCP endpoint for shared deployments, but it does not serve a web application and the repository contains no HTML, CSS, Elm, or other browser assets.

`nexus tui` embeds the terminal renderer and dashboard bundle. It uses a compatible Node runtime already on `PATH` or installs a pinned, verified runtime into `nexus/tui/` under the operating system's per-user cache directory. Dashboard files and the managed runtime do not depend on access to the configured database directory, including when a remote client retains an inaccessible local database path. It requires no Pi installation, agent login, or model credentials for rendering. Remote mode does require the Nexus bearer credential selected by `NEXUS_SECRET` or `client.secret_file`. The Pi extension uses its host's renderer and also observes tool calls and explicit skill invocations through a bounded, fail-open capture queue.

## Why it exists

Agent-facing advisories solve immediate collisions, but a person still needs a compact operational view of repositories, sessions, claims, conflicts, recurring SOS problems, tool activity, skill activity, and memory health. Keeping the interface in the terminal avoids a browser frontend toolchain and browser security surface while remaining available both inside Pi and directly from the CLI. A remote service listener is transport infrastructure shared with every other client, not a separate UI server.

Progressive disclosure keeps recent operational state first. The initial Coordination page answers whether Nexus is healthy, where work is happening, and whether conflicts are open. Full payloads and metadata appear only after a person selects a category and record. Coordination, SOS, Papercuts, Code concerns, Evals, Work, and analytics are read-only; only explicit Memory-tab operations mutate state, and immutable raw notes cannot be edited or deleted.

## Data flow

```mermaid
flowchart TD
    A[Pi /nexus] --> B[Persistent Nexus MCP client]
    C[nexus tui] --> D[Embedded terminal bundle]
    D --> E[System or managed Node runtime]
    E --> B
    F[Pi tool and skill events] --> G[Bounded capture queue]
    G --> B
    B --> H{Configured Nexus client}
    H -->|Local| I[Start or reach Unix-socket daemon]
    H -->|Remote| J[Authenticated /api/v1/request]
    I --> S[NexusService]
    J --> S
    S --> QD[Typed Diesel queries]
    QD --> K[Bounded coordination records]
    QD --> L[Seven-day usage summary]
    QD --> M[Memory context and health]
    QD --> SI[SOS aggregate and source detail]
    K --> N[Typed terminal decoders]
    L --> N
    M --> N
    SI --> N
    N --> O[Coordination, SOS, Tools, Skills, Memory, Papercuts, Code concerns, Evals, Work tabs]
    P[Polling tick] --> Q{Request already active?}
    Q -->|yes| R[Skip overlapping poll]
    Q -->|no| B
    S[Scope change or close] --> T[Abort queued MCP read]
```

## Reading the flowchart

1. **Pi `/nexus`** opens the interface inside an interactive Pi session.
2. **`nexus tui`** opens the same interface directly from the command line with inherited terminal streams.
3. **Embedded terminal bundle** packages the shared dashboard and narrow standalone host in the Rust binary.
4. **System or managed Node runtime** runs the bundle without installing Pi; first use can provision a verified runtime that later works offline.
5. **Persistent Nexus MCP client** is shared by dashboard reads, memory operations, and—inside Pi—analytics capture and first-prompt memory activation.
6. **Bounded capture queue** preserves Pi responsiveness when Nexus is slow or unavailable.
7. **Configured Nexus client** keeps transport choice behind the MCP adapter. No terminal component has a separate remote code path.
8. **Start or reach Unix-socket daemon** preserves bounded local launch and retry behavior.
9. **Authenticated `/api/v1/request`** forwards the same typed operations to the configured service and explicit project ID without starting local state.
10. **NexusService** exposes typed projects, dashboard, usage, and memory requests without presentation concerns.
11. **Typed Diesel queries** read existing projections from SQLite or PostgreSQL; terminal hosts do not duplicate persistence logic.
12. **Bounded coordination records** include exact counts and explicit truncation flags.
13. **Seven-day usage summary** contains tool, script, and skill counts plus capture health.
14. **Memory context and health** remain on the correlated MCP protocol and are delimited as untrusted data when injected into a model session.
15. **SOS aggregate and source detail** is read-only in the terminal: list shows importance and last arrival first, while detail includes source reports, triage timestamps, and lifecycle history. Reporting, resolution, and deprecation remain explicit HTTP, MCP, or CLI actions.
16. **Typed terminal decoders** reject malformed dynamic responses before they enter component state.
17. **Coordination, SOS, Tools, Skills, Memory, Papercuts, Code concerns, Evals, Work tabs** present the shared keyboard-driven component in either host.
18. **Polling tick** refreshes only the active surface and never overlaps a prior polling request.
19. **Skip overlapping poll** bounds work while a previous request is active.
20. **Abort queued MCP read** removes stale scope requests before backpressure can send them.

## Implementation details

`src/persistence/dashboard.rs` owns the read model and keeps database access in Diesel's typed DSL for either backend. `DashboardRecords` returns exact scoped counts alongside bounded record windows. Configuration can request smaller windows, but the server always caps a dashboard at 32 events and 16 sessions, claims, and conflicts. Project discovery remains global but returns at most 16 recently active projects and samples at most four recent sessions for each summary. Truncation is explicit, and hard bounds cannot be raised through `[ui]` configuration.

The shared terminal UI lives under `pi-extension/`. `client.ts` maps typed dashboard, project, and usage reads onto the MCP client. `domain.ts` validates their wire shapes. `navigation.ts` owns Coordination's progressive-disclosure state. `sos.ts` validates SOS list and detail responses and `sos-pane.ts` owns its read-only summary-to-source navigation. `polling.ts` owns independent single-flight dashboard and analytics refresh lifecycles. `usage.ts` and `usage-pane.ts` render seven-day bar summaries. `component.ts` composes the nine tabs.

`connection.ts` opens a responsive connection screen before the first snapshot
arrives in either host. Read-only inspection calls allow twenty seconds at the MCP boundary, leaving
room for the companion’s fifteen-second service deadline and bounded credential
loading. Read adapters explicitly use `inspect`; ordinary calls, including hooks and
writes, retain five seconds. Transport does not infer intent from tool names. A failed
read remains visible there,
with an automatic read retry after two seconds and `r` for an immediate retry.
Only one connection attempt is pending at a time. Quitting or host disposal
aborts client-side reads and retry timers; reads already sent to the service may
finish independently, but late responses cannot open a closed dashboard. Counts appear only
after a real snapshot arrives. These retries apply only to dashboard/project
reads, never to commands, model calls, or Memory writes.

`history-pane.ts` owns the common scoped component lifecycle for Evals and Work:
scope changes cancel pending reads before domain navigation resets, deactivation
clears filter editing, and accepted input/requests notify the renderer. Each
history retains its typed state, navigation, request, and rendering policies.
`text-filter.ts` owns their shared bounded text editing and query application,
including control rejection and Unicode backspace. Work session/activity pages
share one decoder parameterized by the required subject scope.

`mcp-client.ts` owns the persistent correlated child, out-of-order response matching, bounded backpressure, request timeouts, and queued-request cancellation. The child is always a local stdio process, but the Rust `NexusClient` beneath it forwards to the configured Unix socket or authenticated HTTP endpoint. The source `launch-context.ts` defaults to `nexus` on `PATH`; machine setup replaces it in the installed Pi extension with the absolute installed executable and optional absolute explicit config, without embedding a credential. `capture.ts` owns fail-open Pi tool and skill listeners. The memory modules own strict response decoding, activation, prompts, and pane interactions. `index.ts` wires these components into Pi.

`terminal.ts` provides the standalone host using the Pi TUI package alone. `terminal-dialogs.ts` supplies focused input and selection overlays through the shared memory prompt interface, and `theme.ts` defines the small theme contract supported by either host. `pi-extension/build.mjs` bundles that host and its pinned renderer into tracked `pi-extension/dist/nexus-tui.mjs`, including third-party license notices.

All strings received from Nexus, remote peers, filenames, providers, and errors remain unchanged in typed state and storage. Presentation formatters pass them through one terminal display sanitizer immediately before rendering; it replaces C0, DEL, and C1 controls (including ESC, CSI, OSC, OSC-52, BEL, and ST), while multiline detail views preserve only normalized logical line breaks. Trusted theme styling is added after sanitization.

`src/runtime/terminal.rs` embeds the bundle, materializes a content-addressed copy outside the checkout, and replaces the launcher process with the selected runtime. `terminal_runtime.rs` owns compatible-runtime detection and verified installation. Real pseudo-terminal tests run with Pi absent, exercise navigation and memory operations, verify explicit configuration and repository paths containing spaces, and check terminal restoration. Their assertions project the pinned renderer’s cursor-positioning, erasure, styling, and hyperlink output into a bounded current screen. Unchanged rows survive incremental redraws, erased or overwritten text cannot satisfy later assertions, and split escape/UTF-8 sequences retain their meaning; tests never require an unchanged pane to be emitted again. Another scenario proves cached offline startup without Node on `PATH`.

Repository validation installs the single locked Pi-extension Node tree, runs its typecheck and tests, rebuilds the tracked terminal bundle, and then runs the Rust and architecture gates. Installed users need no frontend build tools.

Papercut output, including truncation notices, wraps within the host terminal width.

`papercuts.ts` validates each MCP list page and sends the last row's update-time/ID cursor for older pages. `papercuts-pane.ts` owns project-scoped read-only list/detail navigation in both terminal hosts; it appends each page, retains loaded rows if a later request fails, and preserves selection when the user navigates while a page is loading.

The shared Tools pane offers Activities (default), Capabilities (including inventoried zero-use resources), Opportunities, and Legacy via keys 1–4. Up/down or j/k selects a row, Enter discloses implementations and provenance, n/p pages capabilities or examples, and Escape returns from detail before leaving Tools. Counts distinguish invocations from identified operations; incomplete availability and unclassified work remain explicit. Skills retain their own tab. All reads use the existing bounded poller and abortable page requests.

The read-only Evals tab shares the same component in Pi and the native terminal.
It starts with evaluation sets grouped by suite/target identity, repository, and
model composition. Enter opens the selected set's metric trends; arrows or
`[`/`]` select a metric. Definition, units, calculation, current sample counts,
and the latest literal value accompany the plot. Metric revisions remain
separate series; changing metric meaning cannot silently splice incompatible
values together. Scalar chart approximations do not replace exact caller data,
and opaque/unavailable values remain explicit. Corrections retain history while
superseded results are excluded from current charts. Time plots place dated
observations proportionally; missing values have a question marker. Nexus does
not decide whether an increase or decrease is better.

Evals uses the selected project or current repository, and its header identifies
that actual scope. `b` opens the secondary record browser with at most 24 history
headers per page. Enter fetches an immutable record; `w` follows recorded
why/history and `l` browses server-derived links. Git-backed artifacts display
repository identity, commit, path, and backing metadata; stored blobs display
digest and size. Rendering never fetches artifact bytes or launches an evaluation.

`evals.ts` validates the MCP `nexus_evals` list/get/why envelopes and recursively checks bounded JSON values before presentation; the service owns validation of the closed domain variants. `evals-pane.ts` configures the shared history component. `evals-state.ts` owns the discriminated set/metric/list/detail/history/link navigation state and filter commands; `evals-requests.ts` owns single-flight reads, cancellation, stale-response rejection, and page acceptance. `evals-view.ts` formats structured evidence with terminal sanitization, explicit preview limits, and links. `t` cycles record kinds; `/`, `o`, and `m` edit search, repository, and model filters; `c` clears them. Empty filter input clears that filter. `n` or moving past the last loaded row fetches the next page. Filter and scope changes abort old requests, late responses cannot replace the new view, and failed refreshes keep the last successful data with a stale notice. History truncation and display-only omissions are explicit; unknown or partial capture remains visible in record metadata.

The separate read-only **Code concerns / Problem areas** tab summarizes reported
code problems by feature, subsystem, file, or declaration. Ranked area bars and
a UTC observation trend retain raw observations, known source occurrences,
distinct work/host sessions, and unknown source/context/time counts separately.
`m` cycles sorts by affected work sessions, recent reports, and returns after
resolution. In the recent sort the service orders by source activity while bars
still measure labeled observation counts, never epoch timestamp magnitudes.
Overlapping areas are not additive totals, and no reports means no reported
problems rather than verified code health.

Enter selects an area to reveal its distinct concerns; Enter on a concern opens
a timeline of encounters, remedies, reviews, resolutions, and returns. Arrows
or `j`/`k` select evidence. Metadata below the timeline retains source occurrence,
observation, and receipt times, reporter/session identities, remedy motivation
and outcomes, Git locations, and dirty/verification qualifications. Session names use current metadata matched to the original agent and exact host/work
identities, including additional history pages; missing titles remain explicitly
unavailable. `n` pages areas, concerns, or
immutable observations; `a` browses all concerns, including valid unlocated
reports; Escape returns. An empty area list never hides unlocated concerns or
establishes absence. Partial population/source counts,
bounded metadata previews, next cursors, and source uncertainty remain visible.
The terminal never fetches Git objects, artifact bytes, or executes a remedy.

`concerns.ts` validates bounded `nexus_concerns` projections and JSON evidence;
`concerns-evidence.ts` reads recorded Git locators without inferring identity.
`concerns-state.ts` and `concerns-requests.ts` preserve scoped navigation,
pagination, current selection, aborts, stale reads, and refresh failure behavior.
`concerns-pane.ts` reuses the shared history component; `concerns-view.ts`
sanitizes source text before trusted styling. Both terminal entry points use
the same configured transport and project binding. Papercuts remains the
development/environment friction registry, while SOS represents unusual
situations encountered by agents in running applications; explicit evidence
links retain those separate lifecycle and counting policies.

Evals list rows include occurrence date and time in UTC. Record detail separates the bound receipt from an imported original source receipt. For runs, trials, and attempts, it shows caller-reported lifecycle frontiers above the immutable body, retaining the initial state and exposing unresolved forks or partial history without selecting a winner.

Metric definitions and measurements have their own Evals kind filters. Measurement detail prioritizes the exact stored value and its metric-definition link; labeled relation rows let Enter disclose the definition's meaning, units, value format, calculation, formula, and source evidence. Inline measurement text remains literal, preserving large integers, long decimals, and custom structured formats without numeric parsing or conversions. Structured previews preserve literal null, false, and zero; long previews retain explicit omission notices. Nexus does not execute formulas or choose optimization policy.


The read-only Work tab starts with duration analytics: known session totals and
means, daily duration trends, and means/totals by activity kind. `[`/`]` selects
an activity-kind trend across UTC days. Enter opens a selected parent session's
vertical waterfall, including grandchildren without manually expanding
each branch. Time runs downward, child tasks branch right at their start, and
parallel spans have separate lanes.
The labeled UTC origin is the earliest retained source boundary, including queue
admission before a parent's start. Up/Down or `j`/`k` selects the session root or tasks;
Left selects the recorded parent and Right its first recorded child. Selection
reveals the task in both time and lane windows and highlights its graph span.
Page Up/Down pans time, Shift+Left/Right or `H`/`L` pans lanes, `+`/`-` zooms, and Home/End
jumps to source bounds. The panel immediately below the graph shows the selected
task's recorded kind and display category, state/outcome, source boundaries,
elapsed and queue evidence, executor, parent, gaps, and conflicts. Enter opens
its immutable evidence. Category colors distinguish subagents, tests, scripts,
reviews, tools/commands, and generic work/session spans from recorded kinds.
The leading dot-separated kind family determines its display category; `agent`
and `subagent` both identify subagents. `work.session` and `host.session` are
also recognized explicitly as Session. Custom kinds remain Other rather than
being guessed from titles or executable names. A legend and textual
labels retain these distinctions with `NO_COLOR`. Duration coverage counts
qualify every average; unknown values do not become zeros.
Attributed execution/wait union, unobserved time, and worker occupancy accompany
the graph with missing-boundary and clock/source qualifications. Container
lifetimes do not establish attributed execution. Local capture backlog/loss is
separate from project history. Inclusive activity sums are labeled separately from session elapsed time and
cannot be interpreted as non-overlapping effort or delivery lead time.

The service computes summaries independently of the 24-row record browser,
with explicit source and presentation bounds. Truncated summaries remain partial
and retain coverage notices. Terminal chart windows also disclose omitted points.
The shared `charts.ts` renderer owns width-aware, sanitized bars and time plots;
work/eval adapters retain domain-specific meaning and exact evidence.

`g` opens the same waterfall for any selected session in the paginated source
browser, hierarchy, or session detail. Session names follow recorded host names,
including source-ordered renames; missing names remain explicit host/ID labels.
Coordination session rows use the same authoritative names.

`d` opens the secondary Work record browser and the selected session's recorded
ownership hierarchy. Each attempt remains a separate
row. Right arrow or Space expands direct children in independently paged branches,
including grandchildren across host sessions. Enter opens the selected activity;
`s` opens the session itself, `l` browses attributable links, and `e` pages immutable
observations with occurrence and receipt timestamps, original import receipts,
producer, submitter, and correction references. Activity/session links disclose
one canonical record, so a shared release does not become duplicated execution.
Revision, evaluation, and external links remain visible as retained references.

`t` switches the hierarchy to a relative timeline with a common time axis for
visible intervals. Parallel siblings occupy overlapping positions. Unknown finish
boundaries have a question marker and no invented endpoint; missing origins and
clock anomalies are explicit. Reported elapsed nanoseconds remain exact text,
displayed by decimal placement without floating-point conversion. Inclusive
parent and child durations are not summed into effort or lead-time estimates.
Capture gaps, conflicts, unresolved parentage, bounded results, and stale reads
remain visible. Root timeline rows may retain an unresolved parent ID; they are
not silently reparented in stored data.

`work.ts` validates the typed `nexus_work` read envelopes and shares the bounded
JSON evidence decoder in `wire-values.ts` with Evals before evidence enters UI state. `work-state.ts` traverses only recorded,
expanded ownership with defensive cycle/depth/row bounds. `work-pane.ts` composes pure keyboard navigation from `work-navigation.ts` and
the one abortable request lifecycle in `work-requests.ts` for dashboards, the session list, child pages, detail, and event
pages; superseded scopes or navigation cannot accept late responses. Failed
refreshes retain the previous data with a stale notice. Refresh replaces the first
page of up to 32 expanded branches together; deeper branches can be reopened and
additional pages requested again. `work-view.ts` sanitizes all source text before
terminal rendering in both hosts, without loading artifact bytes or executing
work. Search `/`, repository `o`, host-session `h`, outcome `u`, environment `v`,
and occurrence bounds `a`/`b` filter the selected listing; `c` clears filters.
`f` switches between the primary work-session list and secondary activity search.
Applying a kind filter with `k` selects activity search, because work-session roots
have no activity kind. Activity search includes unlinked observations. Enter opens
an activity's source detail; `w` lists its recorded or derived containing work
sessions and follows a selected session back to its canonical hierarchy. Missing
membership remains explicit. Empty filter input clears the selected filter, and the header identifies
the actual project scope.

Coordination overview counters and records show recent sessions, live claims, and
live unresolved conflicts using the shared persistence liveness predicates. They
do not present accumulated historical open conflicts as current collisions.
Historical records remain available through explicit CLI/MCP all-scope queries.

Dashboard conflict liveness resolves each original owner separately from the
indexed lookup of live leases at its path, including after leases are replaced.
A PostgreSQL scale regression exercises the
complete scoped dashboard with 60,000 historical claims and 257 conflicts inside
a two-second read budget, leaving headroom within the terminal's inspection
deadline. Counts and record windows retain the same exact liveness semantics.

Coordination polling pauses when a different tab is active and resumes on return.
An in-flight client read is aborted and cannot overwrite a later scope. This
avoids running exact coordination counts repeatedly behind the Work or Evals
view. Manual scope refresh and retries retain their existing semantics. Read
deadlines remain bounded and cancel immediately when leaving a view or closing
the terminal; no write is retried by this policy.

The MCP client owns the inspection deadline for Analytics catalog and example
reads too. Their view request owner cancels navigation changes and retains old
data after failure, without imposing a second shorter timeout.
