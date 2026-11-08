// @vitest-environment jsdom
// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import { expect, it } from "vitest";
import { renderChart } from "../src/charts";
import type { ChartInput } from "../src/chart-model";

function render(spans: NonNullable<ChartInput["spans"]>) {
  const host = document.createElement("div");
  renderChart(host, { id: "flow", type: "waterfall", spans }, () => {});
  return host;
}
const root = { executionQualified: true, id: "main", parent: null, label: "Main", kind: "agent", start: 0, end: 60000, queued: null, role: "execution" as const };
const call = { executionQualified: true, returnedAt: 20000, id: "read", parent: "main", label: "Read", kind: "tool", start: 10000, end: 20000, queued: null, role: "execution" as const, callMode: "synchronous" as const };
const returned = (host: HTMLElement, id: string) => host.querySelector(`.flow-return[data-owner="${id}"]`);
const node = (host: HTMLElement, id: string) => host.querySelector(`[data-node-id='${id}']`)!;

it("draws a source-time return and leaves the caller hollow during a synchronous call", () => {
  const host = render([root, call]);
  const parent = node(host, "main"), child = node(host, "read");
  expect(host.querySelector("svg")!.getAttribute("aria-label")).toContain("Time flows down");
  expect(returned(host, "read")?.getAttribute("data-at")).toBe("20000");
  expect(parent.querySelector(".flow-wait")?.getAttribute("data-start")).toBe("10000");
  expect(parent.querySelector(".flow-wait")?.getAttribute("fill")).toBe("none");
  expect(parent.querySelectorAll(".flow-active")).toHaveLength(2);
});

it("does not turn asynchronous child execution into parent waiting or invent its join", () => {
  const host = render([root, { ...call, callMode: "asynchronous", returnedAt: 45000 }]);
  expect(node(host, "main").querySelector(".flow-wait")).toBeNull();
  expect(returned(host, "read")?.getAttribute("data-at")).toBe("45000");
  const unknownJoin = render([root, { ...call, callMode: "asynchronous", returnedAt: null }]);
  expect(returned(unknownJoin, "read")).toBeNull();
});

it("keeps containers and missing finishes unfilled without a false completion", () => {
  const host = render([{ ...root, role: "container" }, { ...call, end: null }]);
  expect(node(host, "main").querySelector(".flow-active")).toBeNull();
  expect(node(host, "main").querySelector(".flow-unknown")).not.toBeNull();
  expect(node(host, "read").querySelector(".flow-active")).toBeNull();
  expect(returned(host, "read")).toBeNull();
  expect(node(host, "read").querySelector(".flow-end")).toBeNull();
});

it("staggered short calls retain individual labels without moving source times", () => {
  const calls = Array.from({ length: 5 }, (_, i) => ({ ...call, id: `call-${i}`, label: `Bash · git diff ${i}`, start: 10000 + i, end: 10000 + i, returnedAt: 10000 + i }));
  const host = render([root, ...calls]);
  const nodes = calls.map(call => node(host, call.id));
  expect(new Set(nodes.map(node => node.getAttribute("data-x"))).size).toBe(5);
  expect(new Set(nodes.map(node => node.querySelector(".flow-label")?.getAttribute("x"))).size).toBe(5);
  for (const [i, child] of nodes.entries()) {
    expect(child.textContent).toContain(calls[i].label);
    expect(child.querySelector(".flow-end")?.getAttribute("data-at")).toBe(String(calls[i].end));
  }
});

it("keeps an async worker's own synchronous wait independent of its parent's state", () => {
  const host = render([root, { ...call, id: "review", kind: "review", end: 50000, callMode: "asynchronous" }, { ...call, id: "nested", parent: "review", start: 22000, end: 26000 }]);
  expect(node(host, "main").querySelector(".flow-wait")).toBeNull();
  expect(node(host, "review").querySelector(".flow-wait")?.getAttribute("data-start")).toBe("22000");
  expect(node(host, "review").querySelectorAll(".flow-active")).toHaveLength(2);
});

it("never draws causal returns for conflicting boundaries or unresolved ancestry", () => {
  const host = render([root, { ...call, executionQualified: false, conflicts: ["conflicting_finishes"] }, { ...call, id: "orphan", parent: "absent" }]);
  expect(returned(host, "read")).toBeNull();
  expect(node(host, "main").querySelector(".flow-wait")).toBeNull();
  expect(returned(host, "orphan")).toBeNull();
  expect(host.textContent).toContain("parent unresolved");
});

it("shows explicit same-executor processing and waits on a container without filling uncovered time", () => {
  const host = render([{ ...root, role: "container", executor: "worker" },
    { ...call, id: "response", callMode: "unknown", executor: "worker" },
    { ...call, id: "await", role: "wait", callMode: "unknown", start: 30000, end: 40000, executor: "worker" }]);
  expect(node(host, "main").querySelector(".flow-active")?.getAttribute("data-start")).toBe("10000");
  expect(node(host, "main").querySelector(".flow-wait")?.getAttribute("data-start")).toBe("30000");
  expect(node(host, "main").querySelectorAll(".flow-unknown")).toHaveLength(3);
});

it("reveals the selected call label when its return edge stretches back to an offscreen parent", () => {
  const scroll = document.createElement("div"); scroll.className = "waterfall-scroll";
  Object.defineProperty(scroll, "clientWidth", { value: 300 });
  const host = document.createElement("div"); scroll.append(host);
  const prototype = SVGElement.prototype as SVGElement & { getBBox?: () => unknown };
  const previous = prototype.getBBox;
  prototype.getBBox = function(this: SVGElement) { return this.classList.contains("flow-label") ? { x: 900, y: 130, width: 140, height: 20 } : { x: 140, y: 100, width: 900, height: 60 }; };
  try {
    renderChart(host, { id: "flow", type: "waterfall", selected: "read", spans: [root, call] }, () => {});
    expect(scroll.scrollLeft).toBe(800);
  } finally { if (previous) prototype.getBBox = previous; else delete prototype.getBBox; }
});

it("retains keyboard focus on the selected activity when Elm redraws the chart", () => {
  const host = render([root, call]); document.body.append(host);
  (node(host, "read") as SVGElement).focus();
  expect(document.activeElement).toBe(node(host, "read"));
  renderChart(host, { id: "flow", type: "waterfall", selected: "read", spans: [root, call] }, () => {});
  expect(document.activeElement).toBe(node(host, "read"));
  host.remove();
});

it("branches at an observed synchronous queue boundary and leaves the caller waiting through execution", () => {
  const host = render([root, { ...call, queued: 5000 }]);
  expect(host.querySelector('.flow-dispatch[data-owner="read"]')?.getAttribute("data-at")).toBe("5000");
  expect(node(host, "main").querySelector(".flow-wait")?.getAttribute("data-start")).toBe("5000");
  expect(node(host, "read").querySelector(".flow-queue")?.getAttribute("data-end")).toBe("10000");
});

it("uses server execution qualification independently of a queue clock anomaly", () => {
  const host = render([root, { ...call, executionQualified: true, gaps: ["queue_wall_clock_anomaly"] }]);
  expect(node(host, "read").querySelector(".flow-active")).not.toBeNull();
  expect(returned(host, "read")).not.toBeNull();
});

it("uses the valid execution start when a queue timestamp is inverted", () => {
  const host = render([root, { ...call, queued: 30000, executionQualified: true, gaps: ["queue_wall_clock_anomaly"] }]);
  expect(host.querySelector('.flow-dispatch[data-owner="read"]')?.getAttribute("data-at")).toBe("10000");
  expect(node(host, "main").querySelector(".flow-wait")?.getAttribute("data-start")).toBe("10000");
});

it("does not reserve the rest of a session as occupied execution for a missing tool finish", () => {
  const spans = Array.from({ length: 20 }, (_, i) => ({ ...call, id: `open-${i}`, label: "rg", start: i * 2000, end: null, returnedAt: null }));
  const host = render([root, ...spans]);
  const xs = spans.map(span => Number(node(host, span.id).getAttribute("data-x")));
  expect(Math.max(...xs) - Math.min(...xs)).toBeLessThanOrEqual(240);
  expect(node(host, "open-0").querySelector(".flow-unknown")).toBeNull();
  expect(node(host, "open-0").querySelector(".flow-open-boundary")).not.toBeNull();
  expect(node(host, "open-0").textContent).toContain("finish unobserved");
});

it("keeps a separated child near its parent when there are no label collisions", () => {
  const host = render([root, { ...call, start: 30000, end: 40000, returnedAt: 40000 }]);
  const distance = Number(node(host, "read").getAttribute("data-x")) - Number(node(host, "main").getAttribute("data-x"));
  expect(distance).toBeLessThanOrEqual(90);
});

it("labels a concrete operation with the same activity bucket and evidence as Tools", () => {
  const host = render([root, { ...call, label: "rg", operations: [{ name: "rg", category_id: "code.search", category_name: "code search", description: "Search source", evidence: "deterministic" }] }]);
  expect(node(host, "read").querySelector(".flow-label")?.textContent).toBe("rg");
  expect(node(host, "read").querySelector(".flow-meta")?.textContent).toContain("code search");
  expect(node(host, "read").getAttribute("aria-label")).toContain("deterministic");
});

it("gives a long sequence enough source-time height instead of spreading labels across dozens of lanes", () => {
  const calls = Array.from({ length: 100 }, (_, i) => ({ ...call, id: `dense-${i}`, label: "rg", start: 5000 + i * 500, end: 5050 + i * 500, returnedAt: 5050 + i * 500 }));
  const host = render([root, ...calls]);
  expect(Number(host.querySelector("svg")!.getAttribute("width"))).toBeLessThanOrEqual(1400);
  expect(Number(host.querySelector("svg")!.getAttribute("height"))).toBeGreaterThan(2000);
  expect(returned(host, "dense-99")?.getAttribute("data-at")).toBe(String(calls[99].end));
});

it("uses the reported duration and does not qualify an unrelated UTC subtraction", () => {
  const host = render([root, { ...call, elapsed: { nanoseconds: "1500000000" } }, { ...call, id: "clock-unknown", executionQualified: false, returnedAt: null }]);
  expect(node(host, "read").querySelector(".flow-meta")?.textContent).toContain("1.5s");
  expect(node(host, "clock-unknown").querySelector(".flow-meta")?.textContent).toContain("duration unqualified");
  expect(returned(host, "clock-unknown")).toBeNull();
});
