# Nexus browser interface

The interface is a read-only Elm application. Elm owns authentication, routes,
scope, filters, pagination, selection, metadata and request generations. D3 draws
SVG bars, temporal plots and the session waterfall through a narrow port adapter.
JavaScript never fetches service data or owns application navigation/state.

```sh
npm ci
npm run check
npm run build
```

Use Node 22.12 or later. The locked Elm compiler fetches the packages pinned in
`elm.json` on first use; retain `~/.elm` (or `ELM_HOME`) for offline subsequent
builds. `npm run dev` serves Vite on loopback and proxies `/ui/api` to a Nexus
development listener at `127.0.0.1:8787`. The listener still enforces configured
browser origins. The deployed application is served by Nexus at `/ui/`.

`npm run build` compiles Elm in optimized mode, packages it and D3 with Vite,
collects runtime license notices and generates `dist/embedded.rs`. The generated
whitelist embeds only the produced files using repository-relative paths. Commit
the complete `dist/` alongside source changes. Native Cargo/release builds use
those validated assets without Node or Elm on the user's machine.

The API consists of session discovery, login/logout and a closed read query
enum. The browser sends `{ project_id, query: { kind, ... } }` to
`POST /ui/api/read`; the backend maps allowed reads to existing typed service
operations. It exposes no machine API bearer credential and no domain writes.
The browser uses its HttpOnly cookie, never local storage. All source text enters
the DOM through Elm text nodes or D3 `.text()`.

`Wire.elm` validates response envelopes and presentation identities. Exact metric
literals stay strings; original response text remains available for immutable
evidence without JSON numeric conversion. D3 receives only display values and
source timestamps, returning chart/record identifiers on selection. Unknown
durations and finishes remain explicit; inclusive nested durations are never
interpreted as additive effort.

Tests cover request generation and retained stale snapshots, malformed response
rejection, exact metric strings, distinct metric revisions, semantic-versus-work
identity, unknown duration coverage, parent/concurrency layout and real SVG
selection with hostile source text.

After authentication, a bounded coordination read retrieves the configured
`ui.refresh_interval_ms` for every route, including a direct Work or Evals URL.
Elm retains that validated interval across navigation and skips overlapping
refreshes. Configuration reads have their own generation; stale reads cannot
revive polling after logout. A failed bootstrap is explicit and Refresh retries it.
