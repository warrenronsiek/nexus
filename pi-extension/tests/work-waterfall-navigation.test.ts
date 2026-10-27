// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { dashboardItems, sessionWaterfall } from "../src/work-dashboard.ts";
import type { WorkDashboard } from "../src/work-dashboard-wire.ts";
import { handleWorkInput } from "../src/work-navigation.ts";
import { WorkRequests } from "../src/work-requests.ts";
import { initialWorkState, newWorkDashboard, type WorkDashboardView } from "../src/work-state.ts";
import { renderWaterfall } from "../src/work-waterfall.ts";
import type { WorkApi, WorkSubject, WorkSummary } from "../src/work.ts";

const root: WorkSummary = { subject: { scope: "session", id: "session" }, title: "Session", kind: "session", work_session_ids: ["session"],
  parent_activity_id: null, has_children: true, state: "finished", outcome: "succeeded", started_at: "2026-10-20T12:00:00Z", finished_at: "2026-10-20T12:02:00Z",
  elapsed: { nanoseconds: "120000000000", method: "monotonic", clock_id: "source" }, conflicts: [], gaps: [], context: { external: [], labels: {} } };
function activity(id: string, parent: string | null, started_at = root.started_at): WorkSummary {
  return { ...root, subject: { scope: "activity", id }, title: id, kind: "test", parent_activity_id: parent, started_at };
}
function data(activities = [activity("parent", null), activity("child", "parent"), activity("sibling", null), activity("orphan", "missing")]): WorkDashboard {
  return { work_session_id: root.subject.id, selected_session: root, sessions: [root], activities,
    session_duration: { count: 1, known_count: 1, total_seconds: 120, average_seconds: 120 }, activity_kinds: [], daily: [], activity_daily: [],
    scanned_subjects: activities.length + 1, source_limit: 4096, truncated: false, notices: [] };
}
function fixture(dashboard = data()) {
  const state = initialWorkState(), view = newWorkDashboard(root.subject.id);
  view.data = dashboard; state.view = view; state.active = true;
  return { state, view };
}
function selected(view: WorkDashboardView): WorkSubject | undefined { return dashboardItems(view)[view.selected]?.subject; }
function press(state: ReturnType<typeof initialWorkState>, ...keys: string[]) { keys.forEach(key => expect(handleWorkInput(state, key).handled).toBe(true)); }
function api(dashboard: WorkApi["dashboard"]): WorkApi {
  return { dashboard, health: vi.fn(async () => ({ available: false })), list: vi.fn(), activities: vi.fn(), timeline: vi.fn(), get: vi.fn(), events: vi.fn() };
}
async function refresh(state: ReturnType<typeof initialWorkState>, view: WorkDashboardView, result: WorkDashboard) {
  new WorkRequests(state, api(vi.fn(async () => result)), vi.fn()).run({ action: "dashboard", view });
  await vi.waitFor(() => expect(state.status).toBe("live"));
}

it("selects the root session initially and opens selected evidence with Enter", () => {
  const { state, view } = fixture();
  expect(selected(view)).toEqual(root.subject);
  expect(handleWorkInput(state, "\r").intent).toEqual({ action: "get", subject: root.subject, presentation: "detail" });
  press(state, "\u001b[B");
  expect(handleWorkInput(state, "\r").intent).toMatchObject({ action: "get", subject: { scope: "activity", id: "parent" } });
  expect(handleWorkInput(state, "s").intent).toMatchObject({ action: "get", subject: root.subject });
});

it("uses arrows and j/k for the same bounded previous and next task selection", () => {
  const { state, view } = fixture();
  press(state, "\u001b[B"); expect(view.selected).toBe(1);
  press(state, "j"); expect(view.selected).toBe(2);
  press(state, "\u001b[A"); expect(view.selected).toBe(1);
  press(state, "k", "k"); expect(view.selected).toBe(0);
  for (let i = 0; i < 8; i++) press(state, "j");
  expect(selected(view)).toEqual({ scope: "activity", id: "orphan" });
});

it("follows recorded parents and first direct children without guessing missing ownership", () => {
  const { state, view } = fixture();
  press(state, "\u001b[C"); expect(selected(view)?.id).toBe("parent");
  press(state, "\u001b[C"); expect(selected(view)?.id).toBe("child");
  press(state, "\u001b[C"); expect(selected(view)?.id).toBe("child");
  press(state, "\u001b[D"); expect(selected(view)?.id).toBe("parent");
  press(state, "\u001b[D"); expect(selected(view)).toEqual(root.subject);
  view.selected = dashboardItems(view).findIndex(item => item.subject.id === "orphan");
  press(state, "\u001b[D"); expect(selected(view)?.id).toBe("orphan");
  expect(state.history).toEqual([]);
});

it("pans time and lanes independently from selection and retains zoom and source-bound jumps", () => {
  const { state, view } = fixture();
  view.waterfall.zoom = 3;
  press(state, "\u001b[6~"); expect(view.waterfall.offset).toBeGreaterThan(0);
  expect(view.selected).toBe(0);
  press(state, "\u001b[5~"); expect(view.waterfall.offset).toBe(0);
  press(state, "\u001b[1;2C"); expect(view.waterfall.laneOffset).toBe(1);
  press(state, "L"); expect(view.waterfall.laneOffset).toBe(2);
  press(state, "\u001b[1;2D", "H"); expect(view.waterfall.laneOffset).toBe(0);
  expect(view.selected).toBe(0);
  press(state, "+"); expect(view.waterfall.zoom).toBe(4);
  press(state, "-"); expect(view.waterfall.zoom).toBe(3);
  press(state, "\u001b[F"); expect(view.waterfall.offset).toBeGreaterThan(0);
  press(state, "\u001b[H"); expect(view.waterfall.offset).toBe(0);
});

it("reveals a selected task in time and lane windows using the latest rendered width", () => {
  const chain = Array.from({ length: 16 }, (_, index) => activity(`deep-${index}`, index ? `deep-${index - 1}` : null, "2026-10-20T12:01:50Z"));
  const { state, view } = fixture(data(chain));
  view.waterfall.zoom = 4;
  renderWaterfall(sessionWaterfall(view.data!), view.waterfall, 28);
  view.selected = dashboardItems(view).length - 2;
  press(state, "\u001b[B");
  expect(selected(view)?.id).toBe("deep-15");
  expect(view.waterfall.offset).toBeGreaterThan(0);
  expect(view.waterfall.laneOffset).toBeGreaterThan(0);
});

it("keeps session and activity scopes distinct when their raw IDs collide", () => {
  const { state, view } = fixture(data([activity(root.subject.id, null), activity("grandchild", root.subject.id)]));
  expect(selected(view)).toEqual(root.subject);
  press(state, "\u001b[C");
  expect(selected(view)).toEqual({ scope: "activity", id: root.subject.id });
  expect(handleWorkInput(state, "\r").intent).toMatchObject({ subject: { scope: "activity", id: root.subject.id } });
  press(state, "\u001b[C"); expect(selected(view)?.id).toBe("grandchild");
  press(state, "\u001b[D"); expect(selected(view)).toEqual({ scope: "activity", id: root.subject.id });
  press(state, "\u001b[D"); expect(selected(view)).toEqual(root.subject);
});

it("preserves the current scoped selection when refresh reorders rows, including input during the read", async () => {
  const { state, view } = fixture();
  view.selected = 2;
  let complete!: (result: WorkDashboard) => void;
  const requests = new WorkRequests(state, api(vi.fn(() => new Promise<WorkDashboard>(resolve => { complete = resolve; }))), vi.fn());
  requests.run({ action: "dashboard", view });
  press(state, "j");
  complete(data([activity("orphan", "missing"), activity("sibling", null), activity("child", "parent"), activity("parent", null)]));
  await vi.waitFor(() => expect(state.status).toBe("live"));
  expect(selected(view)).toEqual({ scope: "activity", id: "sibling" });
  expect(view.selected).toBe(2);
});

it("retains activity scope on refresh and falls back to the root if the selected task disappears", async () => {
  const collision = activity(root.subject.id, null), sibling = activity("sibling", null);
  const { state, view } = fixture(data([collision, sibling]));
  view.selected = 1;
  const load = vi.fn<WorkApi["dashboard"]>().mockResolvedValueOnce(data([sibling, collision])).mockResolvedValueOnce(data([sibling]));
  const requests = new WorkRequests(state, api(load), vi.fn());
  requests.run({ action: "dashboard", view });
  await vi.waitFor(() => expect(state.status).toBe("live"));
  expect(selected(view)).toEqual(collision.subject);
  requests.run({ action: "dashboard", view });
  await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
  await vi.waitFor(() => expect(state.status).toBe("live"));
  expect(selected(view)).toEqual(root.subject);
  expect(view.selected).toBe(0);
});

it("reveals the root fallback after a selected late, deep task disappears", async () => {
  const chain = Array.from({ length: 16 }, (_, index) => activity(`deep-${index}`, index ? `deep-${index - 1}` : null, "2026-10-20T12:01:50Z"));
  const { state, view } = fixture(data(chain));
  view.waterfall.zoom = 4;
  renderWaterfall(sessionWaterfall(view.data!), view.waterfall, 28);
  view.selected = dashboardItems(view).length - 2;
  press(state, "j");
  expect(view.waterfall.laneOffset).toBeGreaterThan(0);
  expect(view.waterfall.offset).toBeGreaterThan(0);
  await refresh(state, view, data([]));
  expect(selected(view)).toEqual(root.subject);
  expect(view.waterfall.laneOffset).toBe(0);
  expect(view.waterfall.offset).toBe(0);
});

it("keeps a retained task visible when newly recorded work changes its source position and lane", async () => {
  const { state, view } = fixture(data([activity("target", null)]));
  view.waterfall.zoom = 4;
  renderWaterfall(sessionWaterfall(view.data!), view.waterfall, 28);
  press(state, "j");
  const previousLane = view.waterfall.laneOffset;
  const siblings = Array.from({ length: 16 }, (_, index) => activity(`sibling-${index}`, null));
  await refresh(state, view, data([...siblings, activity("target", null, "2026-10-20T12:01:50Z")]));
  expect(selected(view)).toEqual({ scope: "activity", id: "target" });
  expect(view.waterfall.laneOffset).toBeGreaterThan(previousLane);
  expect(view.waterfall.offset).toBeGreaterThan(0);
});

it("preserves deliberate time and lane pans when refresh changes metadata without changing placement", async () => {
  const tasks = Array.from({ length: 8 }, (_, index) => activity(`task-${index}`, null));
  const { state, view } = fixture(data(tasks));
  view.waterfall.zoom = 4;
  renderWaterfall(sessionWaterfall(view.data!), view.waterfall, 28);
  press(state, "j", "\u001b[6~", "L");
  const previous = { ...view.waterfall };
  await refresh(state, view, data(tasks.map(task => ({ ...task, title: `${task.title} updated` }))));
  expect(selected(view)).toEqual(tasks[0]!.subject);
  expect(view.waterfall).toEqual(previous);
});
