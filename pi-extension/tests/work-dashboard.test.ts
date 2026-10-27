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
  const api = { dashboard, health: vi.fn<WorkApi["health"]>(async () => ({ available: true, queued: 2, rejected: 1, lost: 3, process_losses: 0, health_failures: 0, last_captured_at: null, last_delivered_at: null })), list: vi.fn(async () => ({ items: [root], next_cursor: null })), timeline: vi.fn(), activities: vi.fn(),
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
  await vi.waitFor(() => expect(screen(pane)).toContain("Session duration breakdown · Ship router"));
  const lines = pane.render(150); const text = lines.join("\n");
  expect(text).toContain("10s");
  expect(text).toContain("Nested QA"); expect(text).toContain("Parallel review");
  expect(text).toContain("All recorded descendants");
  expect(lines.find(line => line.includes("Nested QA start"))).toContain("Parallel review start");
  expect(text).toMatch(/[├┬┼]/u);
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

it("opens every selected session as a vertical waterfall and pans time independently from task selection", async () => {
  const { pane, api } = fixture(); await ready(pane); pane.handleInput("\r");
  await vi.waitFor(() => expect(screen(pane)).toContain("Time ↓"));
  expect(screen(pane)).toContain("Ship router");
  expect(screen(pane)).toContain("● start");
  expect(screen(pane)).toContain("UTC origin");
  pane.handleInput("+");
  const top = screen(pane); pane.handleInput("\u001b[B");
  expect(screen(pane)).not.toBe(top);
  pane.handleInput("j"); pane.handleInput("\r");
  await vi.waitFor(() => expect(api.get).toHaveBeenCalled());
  pane.dispose();
});

it("rejects malformed waterfall evidence before it reaches the terminal layout", async () => {
  const queued = { ...selected.activities[0], id: "bad-queue", executor: null, accounting_role: "execution", queued_at: "not a date" };
  const lossy = { ...selected.activities[0], id: "lossy", executor: null, accounting_role: "execution", elapsed: { nanoseconds: 9007199254740992, method: "monotonic", clock_id: "boot" } };
  for (const intervals of [[{ id: "bad" }], [{ ...selected.activities[0], id: "bad", started_at: "not a date", executor: null, accounting_role: "execution" }], [queued], [lossy]]) {
    const api = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "dashboard", ...selected, intervals })) }, "/repo");
    await expect(api.dashboard(null, "work-1")).rejects.toThrow("unexpected response");
  }
});

it("shows attributed time and missing coverage without equating container lifetime with work", async () => {
  const measured = (seconds: number) => ({ seconds, nanoseconds: String(seconds * 1e9) });
  const accounting = { session_wall: measured(10), occupied_wall: measured(6), inclusive_execution: measured(20), worker_occupancy: measured(15),
    unobserved: measured(4), unobserved_intervals: [], definitions: [], coverage: { activity_count: 3, interval_count: 2, missing_start_count: 0, missing_finish_count: 1,
      clock_conflict_count: 0, conflicted_count: 0, unlinked_count: 0, unknown_executor_count: 1, unknown_role_count: 1, source_partial: false, display_partial: false,
      capability_inventory: "unavailable", observed_adapters: ["codex-native-v1"] } };
  const { pane } = fixture(vi.fn(async (_project: string | null, id?: string) => id ? { ...selected, accounting } : overview));
  await ready(pane); pane.handleInput("\r");
  await vi.waitFor(() => expect(screen(pane)).toContain("Attributed 6s"));
  expect(screen(pane)).toContain("unobserved 4s");
  expect(screen(pane)).toContain("finish missing 1");
  expect(screen(pane)).toContain("inventory unavailable");
  expect(pane.render(60).length).toBeLessThanOrEqual(24);
  pane.dispose();
});

it("shows local capture backlog and loss separately from project accounting", async () => {
  const { pane, api } = fixture(); await ready(pane);
  expect(api.health).toHaveBeenCalledWith(expect.any(AbortSignal));
  expect(screen(pane)).toContain("Local capture · queued 2 · retained rejected 1 · lost 3");
  pane.dispose();
});

it("qualifies queue sums when source clocks cannot measure every queued task", async () => {
  const measured = (seconds: number) => ({ seconds, nanoseconds: String(seconds * 1e9) });
  const accounting = { session_wall: measured(10), occupied_wall: measured(0), inclusive_execution: measured(0), worker_occupancy: measured(0),
    queue_wait: measured(0), unobserved: measured(10), unobserved_intervals: [], definitions: [], coverage: { activity_count: 1, interval_count: 0,
      queue_count: 1, known_queue_count: 0, missing_start_count: 0, missing_finish_count: 0, clock_conflict_count: 1, conflicted_count: 0,
      unlinked_count: 0, unknown_executor_count: 0, unknown_role_count: 0, source_partial: true, display_partial: false,
      capability_inventory: "unavailable", observed_adapters: [] } };
  const { pane } = fixture(vi.fn(async (_project: string | null, id?: string) => id ? { ...selected, accounting } : overview));
  await ready(pane); pane.handleInput("\r");
  await vi.waitFor(() => expect(screen(pane, 240)).toContain("queue sum unknown · 0/1 timed"));
  pane.dispose();
});

it("keeps known delivery backlog while showing unknown losses when the capture ledger is unavailable", async () => {
  const api = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, scope: "local_companion", available: true, work_capture: {
    queued: 2, rejected: 1, capture: null, admission_lost_process: 0, worker_failed_process: 0, health_persistence_failed_process: 1,
  } })) }, "/repo");
  expect(await api.health()).toMatchObject({ available: true, queued: 2, rejected: 1, lost: undefined, health_failures: 1 });
});

it("shows historical capacity loss separately from newer capture-ledger losses", async () => {
  const { pane, api } = fixture();
  api.health.mockResolvedValue({ available: true, queued: 0, rejected: 0, lost: 0, process_losses: 0, health_failures: 0, last_captured_at: null, last_delivered_at: null, legacy_capacity_losses: 7 } as Awaited<ReturnType<typeof api.health>>);
  await ready(pane);
  expect(screen(pane)).toContain("Capacity losses 7");
  pane.dispose();
});

it("shows rejected callback transport separately from quarantined observations", async () => {
  const wire = { ok: true, scope: "local_companion", available: true, work_capture: {
    queued: 0, rejected: 1, capture: { lost: 0, transport_rejected: 4, rejected_replays: 2, last_captured_at: null, last_delivered_at: null },
    admission_lost_process: 0, worker_failed_process: 0, health_persistence_failed_process: 0,
  } };
  const decoded = await new NexusWorkApi({ call: vi.fn(async () => wire) }, "/repo").health();
  expect(decoded).toMatchObject({ rejected: 1, transport_rejected: 4 });
  const { pane, api } = fixture(); api.health.mockResolvedValue(decoded); await ready(pane);
  expect(screen(pane, 240)).toContain("Transport rejected 4");
  expect(screen(pane, 240)).toContain("Callback rejected 2");
  pane.dispose();
});

it("opens the same waterfall for sessions reached through paginated source browsing", async () => {
  const { pane, api } = fixture(); await ready(pane);
  pane.handleInput("d"); await vi.waitFor(() => expect(api.list).toHaveBeenCalled());
  await vi.waitFor(() => expect(screen(pane)).toContain("work sessions loaded"));
  pane.handleInput("g");
  await vi.waitFor(() => expect(api.dashboard).toHaveBeenCalledWith(null, "work-1", expect.any(AbortSignal)));
  await vi.waitFor(() => expect(screen(pane)).toContain("Time ↓"));
  pane.dispose();
});
