// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { expect, it } from "vitest";
import { visibleWidth } from "@earendil-works/pi-tui";
import { layoutWaterfall, renderWaterfall, initialWaterfall, moveWaterfall, revealWaterfall, type WaterfallNode } from "../src/work-waterfall.ts";

const at = (seconds: number) => new Date(Date.UTC(2026, 9, 27) + seconds * 1000).toISOString();
function node(id: string, parent: string | null, start: number | null, end: number | null): WaterfallNode {
  return { id, title: id, kind: "test", parent_activity_id: parent, started_at: start === null ? null : at(start),
    finished_at: end === null ? null : at(end), state: end === null ? "running" : "finished", outcome: end === null ? null : "succeeded",
    elapsed: null, executor: null, conflicts: [], gaps: [] };
}
const root = node("Session name from host", null, 0, 100);
const nodes = [node("Compile", null, 10, 90), node("QA", "Compile", 20, 40), node("Review", "Compile", 20, 60), node("Retry", "Compile", 65, 85)];

it("places parent and parallel children on a common vertical time axis with separate overlapping lanes", () => {
  const layout = layoutWaterfall(root, nodes);
  expect(layout.origin).toBe(Date.parse(at(0)));
  expect(layout.end).toBe(Date.parse(at(100)));
  const byId = new Map(layout.nodes.map(item => [item.node.id, item]));
  expect(byId.get("Compile")!.lane).toBeGreaterThan(byId.get(root.id)!.lane);
  expect(byId.get("QA")!.lane).toBeGreaterThan(byId.get("Compile")!.lane);
  expect(byId.get("Review")!.lane).not.toBe(byId.get("QA")!.lane);
  expect(byId.get("QA")!.start).toBe(byId.get("Review")!.start);
  expect(byId.get("Retry")!.lane).toBe(byId.get("QA")!.lane);
});

it("renders the branching graph with task boundaries and time direction", () => {
  const layout = layoutWaterfall(root, nodes);
  const lines = renderWaterfall(layout, initialWaterfall(), 120);
  expect(lines.join("\n")).toContain("Time ↓");
  expect(lines.join("\n")).toContain("QA");
  expect(lines.join("\n")).toContain("Review");
  expect(lines.join("\n")).toMatch(/[├┬┼]/u);
});

it("pans through elapsed time, zooms without dropping tasks, and reaches the final boundary", () => {
  const layout = layoutWaterfall(root, nodes), view = initialWaterfall();
  moveWaterfall(view, layout, "zoom_in");
  expect(view.zoom).toBe(1);
  moveWaterfall(view, layout, "page_down");
  expect(view.offset).toBeGreaterThan(0);
  moveWaterfall(view, layout, "end");
  expect(renderWaterfall(layout, view, 110).join("\n")).toContain("+100s");
  moveWaterfall(view, layout, "home");
  expect(view.offset).toBe(0);
  expect(layout.nodes).toHaveLength(5);
});

it("keeps missing boundaries and unresolved parents outside claimed complete intervals", () => {
  const layout = layoutWaterfall(root, [node("Missing finish", null, 20, null), node("Missing start", "absent", null, 80)]);
  const text = renderWaterfall(layout, initialWaterfall(), 120).join("\n");
  expect(text).toContain("finish unknown");
  expect(text).toContain("start unknown");
  expect(text).toContain("unresolved parent");
  expect(layout.nodes.find(item => item.node.id === "Missing finish")!.end).toBeNull();
});

it("bounds narrow/deep views and sanitizes labels while retaining every node for horizontal navigation", () => {
  const deep = Array.from({ length: 80 }, (_, i) => node(`deep-${i}\u001b[31m`, i ? `deep-${i - 1}\u001b[31m` : null, i, 100));
  const layout = layoutWaterfall(root, deep), view = initialWaterfall();
  moveWaterfall(view, layout, "right");
  expect(view.laneOffset).toBe(1);
  for (const width of [12, 28, 60, 160]) {
    const lines = renderWaterfall(layout, view, width);
    expect(lines.length).toBeLessThanOrEqual(18);
    expect(lines.every(line => visibleWidth(line) <= width)).toBe(true);
    expect(lines.join("\n")).not.toContain("\u001b");
  }
  expect(layout.nodes).toHaveLength(81);
});

it("retains a full bounded 4096-task parent chain without overflowing the terminal renderer stack", () => {
  const deep = Array.from({ length: 4096 }, (_, i) => node(`task-${i}`, i ? `task-${i - 1}` : null, i, 5000));
  expect(layoutWaterfall(root, deep.reverse()).nodes).toHaveLength(4097);
});

it("shows queue wait before execution without making it a second task or worker interval", () => {
  const build = { ...node("Build", null, 20, 80), queued_at: at(5), queue_elapsed: { nanoseconds: "15000000000", method: "os_monotonic", clock_id: "boot" } };
  const layout = layoutWaterfall(root, [build]);
  expect(layout.nodes).toHaveLength(2);
  const view = initialWaterfall(); moveWaterfall(view, layout, "zoom_in");
  const lines = renderWaterfall(layout, view, 120), text = lines.join("\n");
  expect(text).toContain("Build queued");
  expect(text).toContain("Build start");
  expect(lines.some(line => line.includes("s ") && line.includes("┆"))).toBe(true);
});

it("reserves the root lane when an unresolved child begins after its session ends", () => {
  const layout = layoutWaterfall(node("root", null, 0, 10), [node("orphan", "missing", 20, 30)]);
  const session = layout.nodes.find(item => item.node.id === "root")!;
  const orphan = layout.nodes.find(item => item.node.id === "orphan")!;
  expect(orphan.unresolved).toBe(true);
  expect(orphan.lane).toBeGreaterThan(session.lane);
});

it("labels visible lanes rather than letting offscreen parallel boundaries hide them", () => {
  const parallel = Array.from({ length: 8 }, (_, index) => node(`parallel-${index}`, null, 20, 80));
  const layout = layoutWaterfall(root, parallel), view = initialWaterfall();
  view.laneOffset = layout.nodes.find(item => item.node.id === "parallel-6")!.lane;
  const text = renderWaterfall(layout, view, 60).join("\n");
  expect(text).toContain("parallel-6 start");
  expect(text).not.toContain("parallel-0 start");
});

it("keeps session and activity identities distinct when their raw IDs match", () => {
  const session = node("same-id", null, 0, 100);
  const activity = { ...node("same-id", null, 20, 80), title: "Child sharing root ID" };
  const layout = layoutWaterfall(session, [activity]);
  expect(layout.nodes).toHaveLength(2);
  const child = layout.nodes.find(item => item.node.title === "Child sharing root ID")!;
  expect(child.lane).toBeGreaterThan(0);
  expect(renderWaterfall(layout, initialWaterfall(), 100).join("\n")).toContain("Child sharing root ID");
});

it("reveals the selected task in both axes and marks its span without replacing its type color", () => {
  const parallel = Array.from({ length: 12 }, (_, index) => ({ ...node(`task-${index}`, null, 80, 90), kind: index === 11 ? "agent.task" : "test.rust" }));
  const layout = layoutWaterfall(root, parallel), view = initialWaterfall();
  const selected = { scope: "activity" as const, id: "task-11" };
  renderWaterfall(layout, view, 60);
  moveWaterfall(view, layout, "zoom_in");
  revealWaterfall(view, layout, selected);
  const text = renderWaterfall(layout, view, 60, { selected }).join("\n");
  expect(view.offset).toBeGreaterThan(0);
  expect(view.laneOffset).toBeGreaterThan(0);
  expect(text).toContain("›");
  expect(text).toContain("task-11");
});

it("colors subagent, test, and script spans by recorded kind while keeping source controls inert", () => {
  const tasks = [
    { ...node("Agent\u001b[31m", null, 10, 90), kind: "agent.task" },
    { ...node("Tests", null, 20, 80), kind: "test.rust" },
    { ...node("Script", null, 30, 70), kind: "script" },
  ];
  const colors = { accent: 36, success: 32, warning: 33, muted: 37, dim: 90, error: 31 };
  const theme = { fg: (color: keyof typeof colors, text: string) => `\u001b[${colors[color]}m${text}\u001b[39m`, bold: (text: string) => `\u001b[1m${text}\u001b[22m` };
  const lines = renderWaterfall(layoutWaterfall(root, tasks), initialWaterfall(), 150, { theme });
  const graph = lines.filter(line => /\+\d.*s /u.test(line)).join("\n");
  for (const color of [36, 32, 33]) expect(graph).toContain(`\u001b[${color}m`);
  expect(graph).not.toContain("\u001b[31m");
  expect(graph).toContain("Agent�[31m");
  expect(lines.every(line => visibleWidth(line) <= 150)).toBe(true);
});
