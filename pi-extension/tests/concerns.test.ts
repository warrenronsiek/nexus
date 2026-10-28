// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";
import { ConcernsPane } from "../src/concerns-pane.ts";
import { NexusConcernsApi, type ConcernCounts, type ConcernsApi, type ConcernDashboard, type ConcernDetail } from "../src/concerns.ts";
import type { JsonValue } from "../src/wire-values.ts";

const counts: ConcernCounts = { raw_observations: 9, known_source_occurrences: 4, work_sessions: 3, host_sessions: 5,
  unknown_source_observations: 2, unknown_work_observations: 1, unknown_host_observations: 1, unknown_source_time_observations: 1,
  concerns: 2, returns_after_resolution: 1 };
const area = { id: "router", repository: "repo", kind: "subsystem" as const, key: "router", display_name: "Router fallback", discriminator: null };
const summary = { id: "concern-1", canonical_id: "concern-1", title: "Fallback race", description: "Two callbacks finalize one dispatch.", hypothesis: "Ownership may be unclear", impact: null,
  status: "open" as const, counts, last_observed_at: "2026-10-27T12:00:00Z", created_at: "2026-10-20T12:00:00Z", resolved_at: null, lifecycle_qualification: "fix ancestry unknown", areas: [area] };
const observation = { input: { id: "encounter-1", concern_id: summary.id, occurred_at: "2026-10-27T12:00:00Z", observed_at: "2026-10-27T12:01:00Z",
  context: { host_session_id: "host-1", work_session_id: "work-1", activity_id: "test-1", repository: "repo" },
  body: { event: "remedy_recorded", encounter_ids: ["original"], what: "Serialize completion", why: "Two callbacks observed", outcome: "uncertain", evidence: [] } },
  project_id: "project-1", recorded_at: "2026-10-27T12:01:01Z", provenance: { agent: "codex", session_id: "host-1", model_id: null, config_hash: "config" } };
const dashboard: ConcernDashboard = { project_id: "project-1", query: { sort: "work_sessions" }, counts, areas: [{ area, counts, open_concern_count: 2, last_observed_at: summary.last_observed_at }],
  daily: [{ date: "2026-10-20", raw_observations: 2, known_source_occurrences: 1, work_sessions: 1 }, { date: "2026-10-27", raw_observations: 7, known_source_occurrences: 3, work_sessions: 2 }], next_cursor: null, partial: false, notices: [] };
const detail: ConcernDetail = { summary, history: [observation], next_cursor: null, partial: false };
const theme = { fg: (_color: string, value: string) => value, bold: (value: string) => value };
function fixture() {
  const api: ConcernsApi = { dashboard: vi.fn(async () => dashboard), list: vi.fn(async () => ({ project_id: "project-1", query: { area_id: area.id }, items: [summary], next_cursor: null, total: 1, partial: false })),
    get: vi.fn(async () => detail), history: vi.fn(async () => ({ concern_id: summary.id, observations: [], next_cursor: null })) };
  const pane = new ConcernsPane({ requestRender: vi.fn() } as unknown as TUI, theme, api, vi.fn());
  return { pane, api };
}
const text = (pane: ConcernsPane, width = 140) => pane.render(width).join("\n");

it("starts with ranked code areas, a dated trend, distinct counts and uncertainty", async () => {
  const { pane } = fixture(); pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(text(pane)).toContain("Problem areas");
  expect(text(pane)).toContain("Router fallback");
  expect(text(pane)).toContain("█"); expect(text(pane)).toContain("2026-10-20");
  expect(text(pane)).toContain("9 observations"); expect(text(pane)).toContain("3 work sessions");
  expect(text(pane)).toContain("unknown source 2"); expect(text(pane)).toContain("reported problems");
  pane.dispose();
});

it("selects an area and concern to show its remedy timeline and evidence metadata", async () => {
  const { pane, api } = fixture(); pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(api.list).toHaveBeenCalled());
  await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(api.get).toHaveBeenCalled());
  await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  expect(text(pane)).toContain("Serialize completion"); expect(text(pane)).toContain("Two callbacks observed");
  expect(text(pane)).toContain("work-1"); expect(text(pane)).toContain("fix ancestry unknown");
  expect(text(pane).indexOf("Evidence:")).toBeGreaterThan(text(pane).indexOf("Timeline"));
  for (const width of [1, 18, 60, 120]) expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
  pane.dispose();
});

it("validates scoped bounded MCP reads and rejects malformed recurrence and evidence", async () => {
  const call = vi.fn(async () => ({ ok: true, response: "dashboard", ...dashboard }));
  const api = new NexusConcernsApi({ call }, "/repo");
  expect(await api.dashboard("project-1", { sort: "returns" })).toEqual(dashboard);
  expect(call).toHaveBeenCalledWith("nexus_concerns", { action: "dashboard", project_id: "project-1", query: { sort: "returns" } }, "/repo", undefined);
  call.mockResolvedValue({ ok: true, response: "dashboard", ...dashboard, counts: { ...counts, raw_observations: -1 } });
  await expect(api.dashboard(null, {})).rejects.toThrow("unexpected response");
  const bad = new NexusConcernsApi({ call: vi.fn(async () => ({ ok: true, response: "detail", concern: { ...detail, history: [{ ...observation, input: { ...observation.input, body: { event: "remedy_recorded", text: Infinity } } }] } })) }, "/repo");
  await expect(bad.get(null, summary.id)).rejects.toThrow("unexpected response");
});

it("shows Git location and dirty/verification qualifications separately from long evidence previews", async () => {
  const { pane, api } = fixture();
  const location = { repository: "repo", object_format: "sha1", commit: "a".repeat(40), path: "src/router/fallback.rs", language: "rust", qualified_name: "router::finalize", declaration_kind: "function", line_start: 10, line_end: 20, dirty: true, verification: "supplied" };
  const evidence: JsonValue[] = [{ kind: "external", uri: "https://example.com/long", description: "long context ".repeat(40) }, { kind: "git", location }];
  vi.mocked(api.get).mockResolvedValue({ ...detail, history: [{ ...observation, input: { ...observation.input, body: { ...observation.input.body, evidence } } }] });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  expect(text(pane)).toContain("src/router/fallback.rs"); expect(text(pane)).toContain("dirty true");
  expect(text(pane)).toContain("supplied"); expect(text(pane)).toContain("a".repeat(40));
  pane.dispose();
});

it("sorts recent areas without drawing timestamp magnitudes as observation counts", async () => {
  const { pane, api } = fixture();
  vi.mocked(api.dashboard).mockResolvedValue({ ...dashboard, areas: [dashboard.areas[0]!,
    { ...dashboard.areas[0]!, area: { ...area, id: "other", display_name: "Other area" }, counts: { ...counts, raw_observations: 1 }, last_observed_at: "2026-10-26T12:00:00Z" }] });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("m"); await vi.waitFor(() => expect(api.dashboard).toHaveBeenLastCalledWith("project-1", { sort: "recent", limit: 24 }, expect.any(AbortSignal)));
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  const rows = pane.render(140).filter(line => line.includes("█"));
  expect(rows).toHaveLength(2);
  expect((rows[1]!.match(/█/gu) ?? []).length).toBeLessThan((rows[0]!.match(/█/gu) ?? []).length / 2);
  expect(rows[1]).toContain("1 observations"); pane.dispose();
});

it("retains a stale summary on refresh failure, aborts scope reads and ignores their late data", async () => {
  const { pane, api } = fixture(); pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  vi.mocked(api.dashboard).mockRejectedValueOnce(new Error("offline\u001b[2J"));
  pane.refresh(); await vi.waitFor(() => expect(pane.statusLine()).toContain("stale"));
  expect(text(pane)).toContain("Router fallback"); expect(pane.statusLine()).not.toContain("\u001b");
  let complete!: (value: ConcernDashboard) => void;
  vi.mocked(api.dashboard).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  pane.refresh(); const signal = vi.mocked(api.dashboard).mock.calls.at(-1)![2]!;
  pane.activate("project-2"); expect(signal.aborted).toBe(true);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  complete({ ...dashboard, areas: [{ ...dashboard.areas[0]!, area: { ...area, display_name: "Wrong old project" } }] });
  await Promise.resolve(); expect(text(pane)).not.toContain("Wrong old project"); pane.dispose();
});

it("pages timeline evidence without counting another encounter and selects the appended remedy", async () => {
  const { pane, api } = fixture();
  vi.mocked(api.get).mockResolvedValue({ ...detail, next_cursor: "history-2", partial: true });
  vi.mocked(api.history).mockResolvedValue({ concern_id: summary.id, observations: [{ ...observation, input: { ...observation.input, id: "resolution-2", body: { event: "resolved", reason: "Both completion paths now share an atomic owner" } } }], next_cursor: null });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  pane.handleInput("n"); await vi.waitFor(() => expect(api.history).toHaveBeenCalledWith("project-1", summary.id, "history-2", expect.any(AbortSignal)));
  await vi.waitFor(() => expect(text(pane)).toContain("2 observations loaded"));
  pane.handleInput("j"); expect(text(pane)).toContain("Both completion paths now share an atomic owner");
  expect(text(pane)).toContain("9 observations"); expect(text(pane)).toContain("PARTIAL"); pane.dispose();
});

it("pages areas without changing population counts and keeps source terminal controls inert", async () => {
  const { pane, api } = fixture();
  vi.mocked(api.dashboard).mockResolvedValueOnce({ ...dashboard, next_cursor: "areas-2", partial: true }).mockResolvedValueOnce({ ...dashboard,
    areas: [{ ...dashboard.areas[0]!, area: { ...area, id: "new-area", display_name: "New\u001b]52;c;unsafe\u0007" } }], notices: ["partial\u009b2J"] });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("n"); await vi.waitFor(() => expect(api.dashboard).toHaveBeenLastCalledWith("project-1", { sort: "work_sessions", after: "areas-2", limit: 24 }, expect.any(AbortSignal)));
  await vi.waitFor(() => expect(text(pane)).toContain("2 areas loaded"));
  expect(text(pane)).toContain("9 observations"); expect(text(pane)).not.toContain("18 observations");
  expect(text(pane)).not.toMatch(/[\u001b\u0007\u009b]/u);
  pane.handleInput("j"); expect(text(pane)).toContain("New�]52;c;unsafe�"); pane.dispose();
});

it("discloses capped area mappings without hiding a concern's full recurrence counts", async () => {
  const { pane, api } = fixture(), capped = { ...summary, areas_partial: true };
  vi.mocked(api.list).mockResolvedValue({ project_id: "project-1", query: {}, items: [capped], next_cursor: null, total: 1, partial: false });
  vi.mocked(api.get).mockResolvedValue({ ...detail, summary: capped });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  expect(text(pane)).toContain("area mappings partial");
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  expect(text(pane)).toContain("area mappings partial"); expect(text(pane)).toContain("9 observations"); pane.dispose();
});

it("keeps unlocated concerns visible without claiming that an empty area list means no problems", async () => {
  const { pane, api } = fixture();
  vi.mocked(api.dashboard).mockResolvedValue({ ...dashboard, areas: [] });
  vi.mocked(api.list).mockResolvedValue({ project_id: "project-1", query: {}, items: [{ ...summary, areas: [] }], next_cursor: null, total: 1, partial: false });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(text(pane)).toContain("No code areas reported");
  expect(text(pane)).not.toContain("No reported problems in this retained scope");
  expect(pane.handleInput("a")).toBe(true);
  await vi.waitFor(() => expect(api.list).toHaveBeenCalledWith("project-1", { sort: "work_sessions", limit: 24 }, expect.any(AbortSignal)));
  await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  expect(text(pane)).toContain("All code concerns"); pane.dispose();
});

it("keeps suspected-cause Git attribution distinct from a verified failure location", async () => {
  const { pane, api } = fixture();
  const location = { repository: "repo", object_format: "sha1", commit: "b".repeat(40), path: "src/suspected.rs", language: null, qualified_name: null, declaration_kind: null, line_start: null, line_end: null, dirty: null, verification: "model_suggested" };
  const body = { event: "reported", title: summary.title, description: summary.description, hypothesis: null, impact: null,
    encounter: { symptom: "Wrong dispatch", consequence: null, workaround: null, source: null, areas: [{ area, relation: "suspected_cause", location, memberships: [] }], evidence: [] } };
  vi.mocked(api.get).mockResolvedValue({ ...detail, history: [{ ...observation, input: { ...observation.input, body } }] });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("src/suspected.rs"));
  expect(text(pane)).toContain("role suspected_cause"); expect(text(pane)).toContain("model_suggested");
  expect(text(pane)).toContain("dirty unknown"); pane.dispose();
});

it("shows derived session titles only for the exact original agent and host/work identities", async () => {
  const { pane, api } = fixture();
  const imported = { ...observation, provenance: { ...observation.provenance, agent: "importer" }, source: { project_id: "old", recorded_at: observation.recorded_at, provenance: observation.provenance } };
  const session_titles = [
    { agent: "importer", host_session_id: "host-1", work_session_id: "work-1", host_title: "Wrong importer name", work_title: "Wrong importer work" },
    { agent: "codex", host_session_id: "host-1", work_session_id: "different", host_title: "Wrong work pairing", work_title: "Wrong session" },
    { agent: "codex", host_session_id: "host-1", work_session_id: "work-1", host_title: "Fix router callback", work_title: "Ship router release" },
  ];
  vi.mocked(api.get).mockResolvedValue({ ...detail, history: [imported], session_titles });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  expect(text(pane)).toContain("Fix router callback"); expect(text(pane)).toContain("Ship router release");
  expect(text(pane)).not.toContain("Wrong importer"); expect(text(pane)).not.toContain("Wrong work pairing"); pane.dispose();
});

it("rejects invalid typed query echoes and derived title fields before they reach UI state", async () => {
  const invalidQuery = new NexusConcernsApi({ call: vi.fn(async () => ({ ok: true, response: "dashboard", ...dashboard, query: { sort: "random", repository: 42 } })) }, "/repo");
  await expect(invalidQuery.dashboard(null, {})).rejects.toThrow("unexpected response");
  const invalidTitles = new NexusConcernsApi({ call: vi.fn(async () => ({ ok: true, response: "detail", concern: { ...detail, session_titles: [{ agent: "codex", host_session_id: null, work_session_id: "work", host_title: false, work_title: "name" }] } })) }, "/repo");
  await expect(invalidTitles.get(null, summary.id)).rejects.toThrow("unexpected response");
});

it("keeps exact derived session names when older timeline pages introduce another session", async () => {
  const { pane, api } = fixture();
  vi.mocked(api.get).mockResolvedValue({ ...detail, next_cursor: "older-sessions", partial: true });
  vi.mocked(api.history).mockResolvedValue({ concern_id: summary.id, next_cursor: null,
    observations: [{ ...observation, input: { ...observation.input, id: "older-remedy", context: { ...observation.input.context, host_session_id: "host-older", work_session_id: "work-older" } } }],
    session_titles: [{ agent: "codex", host_session_id: "host-older", work_session_id: "work-older", host_title: "Earlier routing investigation", work_title: "Prepare dispatch safeguards" }] });
  pane.activate("project-1"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("Fallback race"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(text(pane)).toContain("remedy_recorded"));
  pane.handleInput("n"); await vi.waitFor(() => expect(text(pane)).toContain("2 observations loaded"));
  pane.handleInput("j");
  expect(text(pane)).toContain("Earlier routing investigation"); expect(text(pane)).toContain("Prepare dispatch safeguards");
  pane.dispose();
});
