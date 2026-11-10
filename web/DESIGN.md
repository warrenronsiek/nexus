# Nexus developer observability

Nexus is a metrics workbench for developers inspecting agent behavior, session
accounting, evaluation evidence, and recurring problems. Grafana and SigNoz are
the product direction: compact, legible, data first, and useful in long sessions.
The branching source-time session waterfall is the signature, rather than a
marketing illustration or an invented health score.

## Pinned design guidance

[Leonxlnx/taste-skill](https://github.com/Leonxlnx/taste-skill) is vendored unchanged
at `vendor/taste-skill`, commit `12af471f627f866411b4bd785b19646d3672784c`
(MIT). The repo's `.agents/skills/nexus-interface-design` routes UI work to its
app-screen run. The same upstream skill is installed for the current Codex user
as `design-taste-frontend`. It becomes discoverable in a subsequent turn.

The app draw selected **data-terminal**, seed **16077627**, over true-black-flat:
this is a monitoring console read for long sessions. Use Public Sans for labels
and prose, IBM Plex Mono for IDs/code and all figures. This is a deliberate figure
face, not decorative monospace captions. Fonts are pinned Fontsource 5.3.0 assets,
self-hosted in the compiled application with their OFL notices.

Use neutral graphite (`#171719` ground, `#131315` panels), quiet rules (`#2c2c30`),
readable secondary text (`#a2a2ac`), and blue (`#73a9ec`) for chart ink/focus and a
single 1px selection underline. Numbers stay neutral. Panel radius is 4px; no
resting shadows, gradients, tinted icon tiles, glows, or motion. Body is 13px on
desktop and 14px on phone; metadata is 12px and axes at least 11px.

Intentional adaptations of the dealt sheet:

- Nine evidence destinations require a **220px labeled sidebar**, rather than a
  50px icon rail. Below 1100px, Menu folds it into a two/three-column labeled menu.
- Surface colors are neutralized to avoid a violet cast. Rectangular controls
  and panels fit a dense developer workbench rather than pill controls.
- Chart data carries blue ink. Navigation and tabs share the thin selection
  rule; input focus retains a larger accessible outline.
- The waterfall retains four semantic hue families plus neutral: amber agents,
  green tests, blue scripts/tools (two lightness levels), rose reviews, grey
  sessions/other. Labels and the legend provide the independent kind mapping.
- Eval charts do **not** infer a good direction. Caller definitions and exact
  values remain authoritative, including unknown and nonnumeric observations.

## Questions the screens answer

| View | Main question |
| --- | --- |
| Coordination | Which sessions overlapped, and what did Nexus tell each agent? |
| Work | Where does time go, and how did a selected prompt branch into work? |
| Evaluations | How did a named agent measure over time, under which definition? |
| Tools | What semantic activities occurred, which commands performed them, and when? |
| Skills | Which named skills were used in the seven-day window, and what do they say? |
| Code concerns | Which code areas have repeated encounters and remedies? |
| Papercuts | Which development friction is encountered most? |
| SOS | Which unusual in-application situations recur most? |
| Agent memory | What retained knowledge exists, at which summary level? |

### Review-driven layout rules (2026-11-06)

A recorded product review of the live console set these rules:

- A screen answers its question first. No bounded-window caveats, raw JSON,
  search forms, project-ID entry rows or identifier dumps on the page; errors and
  stale snapshots stay explicit.
- Long lists of sessions or evaluation sets become a second picker beside the
  project picker, and the selection lives in the URL.
- A row with a chevron always does something visible: it expands in place.
- A list that repeats a chart above it is removed; the chart rows themselves
  expand (Tools activities, Skills).
- The work tree is rooted at the human prompt. Container spans that describe the
  same prompt are folded, and repeated calls are grouped with a count.
- Eval results are separate observations, so they draw as dots, not a line.
- Date axes step by whole days so a label never repeats.

The Code map uses a single blue treemap inside Code concerns. Area reflects
either the additive measurement or the sample count, as labeled in the map.
Tiles expose exact values on hover and through accessible labels. Selecting a
tile drills into its scope or opens supporting evidence below the map.

## Icons

[Lucide Static](https://lucide.dev/guide/static) **1.66.0** is pinned as a development
source. `src/Icon.elm` contains the selected upstream SVG primitives and a typed
icon API; no runtime icon loader, icon font, or DOM mutation is involved. The
renderer uses a 24px viewBox, 18px outline icons, 1.6px strokes, currentColor,
`aria-hidden=true`, and `focusable=false`. Visible labels or button aria-labels
provide accessible names. Use icons for navigation, search, refresh, sign-out,
back/forward, zoom, and empty records. Data marks are still chart geometry.

To add an icon, copy its primitives from the locked
`node_modules/lucide-static/icons/<name>.svg` into typed `Svg.path`, `Svg.circle`,
or other `elm/svg` constructors in the Icon module, retaining its coordinates and
upstream license. Update the icon provenance if the pinned version changes.
The build collects all runtime notices through one package inventory, including
Lucide's ISC/Feather MIT notices and Elm SVG's BSD license, in
`dist/THIRD_PARTY_LICENSES.txt`.

## Validation

Run `npm run check` and `npm run build` from `web`, then stage the generated bundle
before `./scripts/check.sh` (which verifies deterministic assets). The actual Elm
browser test covers decorative SVG navigation, unchanged names/current route,
menu expansion and closure on navigation, nine read-only screens, selected work
metadata, exact eval values, project scope, and refresh behavior.

Use an isolated fixture service for browser QA; never seed sample observations
into a user's live repository. Check desktop, tablet and 390px phone layouts,
including navigation, work charts/waterfall selection, eval definitions/exact
values, long identifiers and empty states. Keep overflowing waterfall/chart
content inside its own viewport. The skill's static app linter can run with
`node vendor/taste-skill/scripts/lint.mjs --app web/src`; its monospace warnings are
the intentional figure face above. Use the approved computer-use tool for live
screenshots instead of the upstream raw-CDP inspector when host rules require it.

### Recorded visual review (2026-11-01)

Safari responsive mode inspected actual compiled Elm/D3 against an isolated
SQLite fixture service at 1440×1000, 768×1024, and 390×844. Fixture data was a
15-minute router session with seven activities (including concurrent tests,
build and review), plus three caller-owned latency runs of 10, 8.5 and 7 seconds.
This data is QA-only; production continues to read the configured Nexus service.
The pre-change browser showed the navy/mint marketing login and geometric nav.

The seven app-screen critique questions, after the second visual pass:

1. Work's summary and paired duration charts answer duration questions; the
   selected waterfall becomes the largest block. Evals puts its selected metric
   trend next to its definition and exact results, without an improvement claim.
2. Summary cells and panels align to the same edges. Charts stack at narrow
   widths; their panels keep their natural height rather than stretching blanks.
3. Graphite chrome stays neutral; blue chart ink and the labeled semantic task
   palette carry data. The mono figure face is visible in the summary strip.
4. The session total and average are both 15m for the single sample (1/1); the
   duplicate session row agrees. Six activity kinds describe seven activities.
   Inclusive activity sums are explicitly not compared to wall-time totals.
5. Unknown and missing evidence retain their existing words and exact source
   metadata; real zero counts remain zero. No decorative change badges were added.
6. Waterfall swatches match their CSS-token SVG spans. Fixed the first pass's
   clipped bar values by placing right-aligned values above thin bars. Fixed a
   single observation showing five repeated date ticks; it now has one date tick.
7. Phone navigation folds behind Menu; all nine labels/icons remain available.
   Selection closes the menu. The waterfall scrolls within its own viewport and
   keyboard selection updates the metadata below. Safari selects now have proper
   control height and a Lucide chevron rather than an undersized native appearance.

Static taste lint reports zero failures; its four monospace warnings are the
intentional numeric/code face. The raw-CDP inspector was not used because this
host requires the computer-use tool for browser interaction. No live records,
credentials, or browser screenshots are committed with these notes.

### Execution flow

Time flows vertically down. Dispatches branch out and observed results return;
independent workers keep their own activity/wait states. Horizontal collision
packing separates call identities and labels without shifting source times. Small
label leaders allow near-instant operations to retain readable names. The session
envelope uses a quiet margin rather than another full label column. Filled task
ink means qualified observed execution; hollow dots mean waiting/queued; grey
hollow dashes mean unknown state/finish. Known completion has a terminal cap.
Missing model boundaries remain unobserved, including gaps between tool calls.
D3 owns chart geometry and SVG; Elm owns controls, zoom/selection state, requests,
and evidence. The chart has local scrolling and a Fit control, with no animation
or new browser mutations. The original horizontal Gantt renderer is replaced.

### Execution-flow validation (2026-11-07)

The first public-renderer regression run (`node web/node_modules/vitest/vitest.mjs
run web/tests/execution-flow.test.ts --root web`) failed all six original cases:
missing returns, filled caller waits, missing async independence, and unstaggered
calls. The same focused command passed after implementation; the retained suite
now also exercises queue boundaries, evidence qualification, and keyboard reveal
and focus. `cargo test --lib tool_flow_retains_safe_operations -- --nocapture`
failed before capture retained invocation semantics, then passed. The dashboard
projection regression failed on `Bash` versus `Bash · cargo test`, then passed.

Original-checkout validation: 20 Vitest and 7 Elm tests pass, the production build passes,
and the default Rust all-targets suite passes 681 tests (one opt-in test ignored).
SQLite all-targets and PostgreSQL server Clippy pass with warnings denied.
Feature annotations and whitespace checks pass. The taste linter reports zero
failures; its four monospace warnings remain the documented numeric/code face.
The canonical `./scripts/check.sh` stops at unrelated repository complexity
violations; no full-gate success is claimed. The changed flow files pass their
scoped complexity checks. The former horizontal renderer and its row layout
were removed; no parallel display path remains.

An isolated illustrative fixture exercised the compiled Elm/D3 application at
1440×1000, 768×1024 and 390×844. The page had no horizontal overflow; the chart
retained its own scroll area, state legend, Fit/zoom, exact task selection and
phone keyboard label reveal. The fixture does not seed or modify live history.
A metadata-only layout benchmark placed 512 simultaneous calls in 44 ms versus
704 ms before spatial indexing, and the full 4096-node source bound in 2.54 s
on this machine. This is a layout benchmark, not a rendering or production SLA.

Delivery port: the current-main candidate passes the canonical gate with CI's
pinned BCA 2.2.0 (the previous checkout used BCA 2.5.1). Advisory review prompted
server-owned execution qualification, queue-clock regression coverage, removal
of grouped-node residue, and cached label obstacles. The queue anomaly and
ambiguous-operation regressions were observed red before their fixes passed.

### Execution-flow audit follow-up

Concrete command labels and activity buckets reuse Tools' persisted classification
by exact invocation identity; selection preserves evidence labels. An unobserved
execution finish is an open source-boundary marker rather than a ribbon extended
to the session horizon. Its label states `finish unobserved`. Container envelopes
retain their separate unknown-state meaning. Collision packing reserves only
observed task geometry, with 72px initial child offsets and extra space for actual
label conflicts. D3 remains the visualization owner; Elm retains application and
selection state.
