// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";
import { EvalsPane } from "../src/evals-pane.ts";
import type { EvalDashboard, EvalPoint, EvalSeries } from "../src/evals-dashboard.ts";
import { NexusEvalsApi } from "../src/evals.ts";

const theme = { fg: (_color: string, text: string) => text, bold: (text: string) => text };
const groups = [{ id: "routing-a", label: "Routing reliability", suite_identity: "Routing reliability", suite_revision_ids: ["suite-v1", "suite-v2"],
  target_identity: "Router and specialists", target_revision_ids: ["target-v1"], components: ["router", "specialist"], repository: ["nexus"], models: ["model-a", "model-b"],
  latest_at: "2026-10-23T12:00:00Z", statuses: { completed: 3 }, run_count: 3, result_count: 4 }];
const points: EvalPoint[] = [10, 7].map((value, index) => ({ record_id: `measurement-${index}`, run_id: `run-${index}`, suite_revision_id: "suite-v1", target_revision_id: "target-v1",
  occurred_at: `2026-10-${22 + index}T12:00:00Z`, value: { storage: "inline", text: String(value) }, numeric_value: value, replaces: null }));
const series: EvalSeries[] = [{ id: "latency-v1", label: "P95 duration", unit: "seconds", subject_scope: "run", definition_id: "metric-v1", superseded_count: 0,
  description: "Elapsed end-to-end time", calculation: "p95(completed durations)", points },
{ id: "cost-v1", label: "Total cost", unit: "USD", subject_scope: "run", definition_id: "metric-v2", description: "Caller reported cost", superseded_count: 0,
  calculation: "sum(receipts)", points: [{ ...points[0], value: { storage: "inline", text: "900719925474099312345678901234567890" }, numeric_value: null }] }];
const dashboard: EvalDashboard = { groups, selected_group_id: null, series: [], run_count: 3, result_count: 4, scanned_records: 5, source_limit: 4096, truncated: false, notices: [] };
const selected: EvalDashboard = { ...dashboard, selected_group_id: groups[0].id, series };
function screenText(pane: InstanceType<typeof EvalsPane>, width = 110): string { return pane.render(width).join("\n"); }
function harness(load = vi.fn(async (_project: string | null, groupId?: string) => groupId ? selected : dashboard)) {
  const api = { dashboard: load, list: vi.fn(async () => ({ records: [], next_cursor: null })), get: vi.fn(), why: vi.fn() };
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme, api, vi.fn());
  return { pane, api };
}

it("starts with eval set summaries and keeps immutable records as a secondary view", async () => {
  const { pane, api } = harness();
  pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  const screen = screenText(pane);
  expect(screen).toContain("Evaluation sets");
  expect(screen).toContain("Routing reliability");
  expect(screen).toContain("3 runs");
  expect(screen).toContain("model-a + model-b");
  expect(api.list).not.toHaveBeenCalled();
  pane.handleInput("b");
  await vi.waitFor(() => expect(screenText(pane)).toContain("records loaded"));
  pane.dispose();
});

it("charts selected metric definitions separately and preserves exact opaque results", async () => {
  const { pane, api } = harness();
  pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(api.dashboard).toHaveBeenLastCalledWith("project-1", "routing-a", expect.any(AbortSignal));
  const screen = screenText(pane);
  expect(screen).toContain("P95 duration");
  expect(screen).toContain("seconds");
  expect(screen).toContain("metric-v1");
  expect(screen).toContain("p95(completed durations)");
  expect(screen).toMatch(/[█●•]/u);
  expect(screen).not.toContain("Total cost");
  pane.handleInput("]");
  const opaque = screenText(pane);
  expect(opaque).toContain("Total cost");
  expect(opaque).toContain("900719925474099312345678901234567890");
  expect(opaque).toContain("No numeric chart");
  expect(opaque).not.toContain("metric-v1");
  for (const width of [1, 20, 40, 80]) expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
  pane.handleInput("\u001b");
  expect(screenText(pane)).toContain("Evaluation sets");
  pane.dispose();
});

it("retains dashboard on failed refresh and cancels superseded composition and scope reads", async () => {
  let finish!: (value: typeof selected) => void;
  const delayed = new Promise<typeof selected>(resolve => { finish = resolve; });
  const load = vi.fn().mockResolvedValueOnce(dashboard).mockReturnValueOnce(delayed)
    .mockResolvedValueOnce({ ...dashboard, groups: [{ ...groups[0], suite_identity: "Different project", label: "Different project" }] }).mockRejectedValueOnce(new Error("offline"));
  const { pane } = harness(load);
  pane.activate("old");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  const signal = load.mock.calls[1][2] as AbortSignal;
  pane.deactivate();
  expect(signal.aborted).toBe(true);
  pane.activate("new");
  await vi.waitFor(() => expect(screenText(pane, 100)).toContain("Different project"));
  finish(selected); await delayed;
  expect(screenText(pane, 100)).not.toContain("Routing reliability");
  pane.refresh();
  await vi.waitFor(() => expect(pane.statusLine()).toContain("stale"));
  expect(screenText(pane, 100)).toContain("Different project");
  pane.dispose();
});

it("decodes bounded dashboard data and rejects non-finite graph values and mismatched selected groups", async () => {
  const call = vi.fn(async () => ({ ok: true, action: "dashboard", ...selected }));
  const api = new NexusEvalsApi({ call }, "/repo");
  expect((await api.dashboard("project-1", "routing-a")).series[1].points[0].value).toEqual(series[1].points[0].value);
  expect(call).toHaveBeenCalledWith("nexus_evals", { action: "dashboard", project_id: "project-1", group_id: "routing-a" }, "/repo", undefined);
  call.mockResolvedValueOnce({ ok: true, action: "dashboard", ...selected, selected_group_id: "other" });
  await expect(api.dashboard(null, "routing-a")).rejects.toThrow("unexpected response");
  call.mockResolvedValueOnce({ ok: true, action: "dashboard", ...selected, series: [{ ...series[0], points: [{ ...points[0], numeric_value: Infinity }] }] });
  await expect(api.dashboard(null, "routing-a")).rejects.toThrow("unexpected response");
});

it("summarizes opaque categorical results by exact value without treating them as numeric scores", async () => {
  const categorical = { ...series[1], label: "Routing outcome", unit: "caller labels", points: ["routed", "routed", "blocked"].map((text, index) => ({
    ...points[0], record_id: `label-${index}`, value: { storage: "inline" as const, text }, numeric_value: null,
  })) };
  const { pane } = harness(vi.fn(async (_project: string | null, groupId?: string) => groupId ? { ...selected, series: [categorical] } : dashboard));
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  const screen = screenText(pane, 120);
  expect(screen).toContain("Exact-value frequencies");
  expect(screen).toMatch(/routed\s+█+\s+2/u);
  expect(screen).toMatch(/blocked\s+█+\s+1/u);
  expect(screen).not.toContain("better");
  pane.dispose();
});

it("keeps distinct agent compositions selectable and applies overview filters without fetching records", async () => {
  const swapped = { ...groups[0], id: "routing-b", components: ["router (dispatch) · declared model-b [fixture:model-b] · nexus", "worker (answer) · declared model-a [fixture:model-a] · nexus"] };
  const summary = { ...dashboard, groups: [groups[0], swapped], run_count: 6 };
  const load = vi.fn(async (_project: string | null, id?: string) => ({ ...summary, selected_group_id: id ?? null, series: id ? series : [] }));
  const { pane, api } = harness(load);
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("j"); pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(load).toHaveBeenLastCalledWith(null, "routing-b", expect.any(AbortSignal));
  expect(screenText(pane)).toContain("worker (answer) · declared model-a [fixture:model-a] · nexus");
  pane.handleInput("\u001b"); pane.handleInput("o"); pane.handleInput("another-repo"); pane.handleInput("\r");
  expect(screenText(pane)).toContain("No matching evaluation sets");
  expect(api.list).not.toHaveBeenCalled();
  pane.handleInput("c");
  expect(screenText(pane)).toContain("2 matching sets");
  pane.dispose();
});

it("shows a metric formula before long calculation metadata in the summary", async () => {
  const formula = { ...series[0], calculation: JSON.stringify({ description: "Caller timing evidence ".repeat(12), formula: "p95(elapsed_seconds)", source_artifacts: ["pinned-code"] }) };
  const { pane } = harness(vi.fn(async (_project: string | null, id?: string) => id ? { ...selected, series: [formula] } : dashboard));
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(screenText(pane)).toContain("p95(elapsed_seconds)");
  pane.dispose();
});

it("cancels a pending set read before local filters can exclude its composition", async () => {
  let finish!: (value: EvalDashboard) => void;
  const delayed = new Promise<EvalDashboard>(resolve => { finish = resolve; });
  const load = vi.fn().mockResolvedValueOnce(dashboard).mockReturnValueOnce(delayed);
  const { pane } = harness(load);
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  const signal = load.mock.calls[1][2] as AbortSignal;
  pane.handleInput("o"); pane.handleInput("another-repo"); pane.handleInput("\r");
  expect(screenText(pane)).toContain("0 matching sets");
  expect(signal.aborted).toBe(true);
  finish(selected); await delayed;
  expect(screenText(pane)).toContain("0 matching sets");
  expect(screenText(pane)).toContain("All captured history");
  expect(screenText(pane)).not.toContain("Routing reliability · results over time");
  pane.dispose();
});

it("cancels pending set selection when clearing overview filters", async () => {
  let finish!: (value: EvalDashboard) => void;
  const delayed = new Promise<EvalDashboard>(resolve => { finish = resolve; });
  const load = vi.fn().mockResolvedValueOnce(dashboard).mockReturnValueOnce(delayed);
  const { pane } = harness(load);
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("o"); pane.handleInput("nexus"); pane.handleInput("\r");
  pane.handleInput("\r");
  const signal = load.mock.calls[1][2] as AbortSignal;
  pane.handleInput("c");
  expect(signal.aborted).toBe(true);
  finish(selected); await delayed;
  expect(screenText(pane)).toContain("Evaluation sets");
  expect(screenText(pane)).not.toContain("Routing reliability · results over time");
  pane.dispose();
});

it("keeps rich result history, metric meaning and coverage visible in a compact terminal", async () => {
  const richGroup = { ...groups[0], components: ["router", "planner", "worker", "reviewer"] };
  const manyPoints = Array.from({ length: 512 }, (_, index) => ({ ...points[0], record_id: `point-${index}`,
    occurred_at: new Date(Date.UTC(2026, 9, 22, 12, index)).toISOString(),
    value: { storage: "inline" as const, text: index % 2 ? "7" : "10" }, numeric_value: index % 2 ? 7 : null, replaces: index === 511 ? "previous-result" : null }));
  const rich = { ...selected, groups: [richGroup], series: [{ ...series[0], points: manyPoints, definition_id: "metric-definition-" + "a".repeat(80), superseded_count: 3 }],
    truncated: true, notices: ["source_limit: bounded scan", "display_limit: recent results", "Metric revisions remain separate"] };
  const { pane } = harness(vi.fn(async (_project: string | null, id?: string) => id ? rich : { ...dashboard, groups: [richGroup] }));
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  const lines = pane.render(60);
  expect(lines.length).toBeLessThanOrEqual(24);
  expect(lines.join("\n")).toContain("seconds");
  expect(lines.join("\n")).toContain("scope: run");
  expect(lines.join("\n")).toContain("p95(completed durations)");
  expect(lines.join("\n")).toContain("Partial history");
  expect(lines.join("\n")).toMatch(/Latest .*: 7/u);
  expect(lines.join("\n")).toContain("b records");
  expect(lines.every(line => visibleWidth(line) <= 60)).toBe(true);
  pane.dispose();
});
