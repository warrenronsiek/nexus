// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";
import { WorkPane } from "../src/work-pane.ts";
import { NexusWorkApi, type WorkApi, type WorkSummary } from "../src/work.ts";

const context = { external: [], labels: {} };
const root: WorkSummary = { subject: { scope: "session", id: "work-1" }, title: "Ship router", kind: "work.session", work_session_ids: ["work-1"],
  parent_activity_id: null, has_children: true, state: "finished", outcome: "succeeded", started_at: "2026-10-20T12:00:00Z", finished_at: "2026-10-20T12:00:10Z",
  elapsed: { nanoseconds: "10000000000", method: "monotonic", clock_id: "root-clock" }, conflicts: [], gaps: [], context };
function child(id: string, parent: string | null, start: number, end: number): WorkSummary {
  return { ...root, subject: { scope: "activity", id }, title: id, kind: "test", parent_activity_id: parent,
    started_at: `2026-10-20T12:00:0${start}Z`, finished_at: `2026-10-20T12:00:0${end}Z`,
    elapsed: { nanoseconds: String((end - start) * 1e9), method: "monotonic", clock_id: id } };
}
const duration = { count: 2, known_count: 1, total_seconds: 10, average_seconds: 10 };
const overview = { work_session_id: null, selected_session: null, sessions: [root], activities: [], session_duration: duration,
  activity_kinds: [{ kind: "test", count: 3, known_count: 2, total_seconds: 8, average_seconds: 4 }],
  daily: [{ date: "2026-10-20", session_count: 1, known_count: 1, total_seconds: 10, average_seconds: 10 },
    { date: "2026-10-21", session_count: 1, known_count: 0, total_seconds: 0, average_seconds: null }], truncated: false, notices: [], scanned_subjects: 5, source_limit: 4096, activity_daily: [{ date: "2026-10-20", kind: "test", count: 2, known_count: 2, total_seconds: 8, average_seconds: 4 }] };
const selected = { ...overview, work_session_id: "work-1", selected_session: root,
  activities: [child("Implementation", null, 1, 8), child("Nested QA", "Implementation", 2, 5), child("Parallel review", "Implementation", 2, 5)] };
const theme = { fg: (_color: string, text: string) => text, bold: (text: string) => text };
function fixture(dashboard: WorkApi["dashboard"] = vi.fn(async (_project: string | null, id?: string) => id ? selected : overview)) {
  const api = { dashboard, list: vi.fn(async () => ({ items: [root], next_cursor: null })), timeline: vi.fn(), activities: vi.fn(),
    get: vi.fn(async () => ({ summary: root, observations: [], links: [], truncated: false })), events: vi.fn() };
  const pane = new WorkPane({ requestRender: vi.fn() } as unknown as TUI, theme, api, vi.fn());
  return { pane, api };
}
const screen = (pane: WorkPane, width = 120) => pane.render(width).join("\n");
async function ready(pane: WorkPane) { pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live")); }

it("opens work with qualified duration charts over history and session selection", async () => {
  const { pane, api } = fixture(); await ready(pane);
  expect(api.dashboard).toHaveBeenCalledWith(null, undefined, expect.any(AbortSignal));
  expect(api.list).not.toHaveBeenCalled();
  const text = screen(pane);
  expect(text).toContain("Work duration dashboard");
  expect(text).toContain("Session durations over time");
  expect(text).toContain("Average activity duration by kind");
  expect(text).toContain("2/3 timed");
  expect(text).toContain("1/2 sessions timed");
  expect(text).toContain("inclusive");
  expect(text).toContain("Ship router");
  expect(text).toContain("2026-10-20");
  expect(text).toContain("2026-10-21");
  pane.dispose();
});

it("opens all descendant intervals as a visual session breakdown without manual expansion", async () => {
  const { pane, api } = fixture(); await ready(pane); pane.handleInput("\r");
  await vi.waitFor(() => expect(api.dashboard).toHaveBeenCalledWith(null, "work-1", expect.any(AbortSignal)));
  await vi.waitFor(() => expect(screen(pane)).toContain("Session duration breakdown"));
  const lines = pane.render(150); const text = lines.join("\n");
  expect(text).toContain("10s");
  expect(text).toContain("Nested QA"); expect(text).toContain("Parallel review");
  expect(text).toContain("All recorded descendants");
  const bars = lines.filter(line => /Nested QA|Parallel review/u.test(line)).join("\n").match(/\[[ =?]+\]/gu) ?? [];
  expect(bars[0]).toBeDefined(); expect(bars[0]).toBe(bars[1]);
  expect(api.timeline).not.toHaveBeenCalled();
  for (const width of [18, 40, 80]) expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
  pane.dispose();
});

it("preserves charts on refresh failure, discloses caps, and offers secondary record browsing", async () => {
  const dashboard = vi.fn().mockResolvedValueOnce({ ...overview, truncated: true, notices: ["History cap reached"] }).mockRejectedValueOnce(new Error("offline"));
  const { pane, api } = fixture(dashboard); await ready(pane);
  expect(screen(pane)).toContain("Partial history"); expect(screen(pane)).toContain("History cap reached");
  pane.refresh(); await vi.waitFor(() => expect(pane.statusLine()).toContain("stale"));
  expect(screen(pane)).toContain("Average activity duration by kind");
  pane.handleInput("d"); await vi.waitFor(() => expect(api.list).toHaveBeenCalled());
  expect(screen(pane)).toContain("Work sessions"); pane.dispose();
});

it("aborts old dashboard scopes and ignores their late data", async () => {
  let complete!: (value: typeof overview) => void;
  const dashboard = vi.fn().mockImplementationOnce(() => new Promise<typeof overview>(resolve => { complete = resolve; })).mockResolvedValue(overview);
  const { pane } = fixture(dashboard); pane.activate("old"); const oldSignal = dashboard.mock.calls[0][2] as AbortSignal;
  pane.activate("new"); expect(oldSignal.aborted).toBe(true); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  complete({ ...overview, sessions: [{ ...root, title: "Stale old data" }] }); await Promise.resolve();
  expect(screen(pane)).not.toContain("Stale old data"); pane.dispose();
});

it("decodes dashboard responses with scope identity and finite qualified metrics", async () => {
  const call = vi.fn(async () => ({ ok: true, response: "dashboard", ...overview }));
  const api = new NexusWorkApi({ call }, "/repo");
  expect((await api.dashboard("project-1")).sessions[0]?.title).toBe("Ship router");
  expect(call).toHaveBeenCalledWith("nexus_work", { action: "dashboard", project_id: "project-1" }, "/repo", undefined);
  const wrong = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "dashboard", ...selected, work_session_id: "wrong" })) }, "/repo");
  await expect(wrong.dashboard(null, "work-1")).rejects.toThrow("unexpected response");
  const malformed = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "dashboard", ...overview, session_duration: { ...duration, average_seconds: Infinity } })) }, "/repo");
  await expect(malformed.dashboard(null)).rejects.toThrow("unexpected response");
});

it("renders truncated duration rows without leaking terminal reset sequences", async () => {
  const dashboard = vi.fn(async () => ({ ...overview, sessions: [{ ...root, title: "Long session title ".repeat(20) }] }));
  const { pane } = fixture(dashboard); await ready(pane);
  expect(screen(pane, 40)).not.toContain("�[0m");
  pane.dispose();
});

it("keeps the overview and parent picker visible in a compact terminal", async () => {
  const dashboard = vi.fn(async () => ({ ...overview,
    sessions: Array.from({ length: 10 }, (_, index) => ({ ...root, subject: { scope: "session" as const, id: `work-${index}` } })),
    activity_kinds: Array.from({ length: 8 }, (_, index) => ({ ...overview.activity_kinds[0]!, kind: `activity.kind.${index}` })),
    truncated: true, notices: ["Source history cap reached", "Display cap reached", "Other qualification"] }));
  const { pane } = fixture(dashboard); await ready(pane);
  const lines = pane.render(60);
  expect(lines.length).toBeLessThanOrEqual(24);
  expect(lines.join("\n")).toContain("Parent sessions");
  pane.dispose();
});
