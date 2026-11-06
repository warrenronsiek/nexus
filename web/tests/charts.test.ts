// @vitest-environment jsdom
// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import { expect, it } from "vitest";
import { layoutWaterfall, renderChart } from "../src/charts";

  it("branches children right, preserves source positions, and separates concurrent siblings", () => {
    const layout = layoutWaterfall([
      { id: "root", parent: null, label: "Session", kind: "session", start: 1000, end: 8000, queued: null },
      { id: "a", parent: "root", label: "Review", kind: "review", start: 3000, end: 6000, queued: 2000 },
      { id: "b", parent: "root", label: "Tests", kind: "test", start: 4000, end: 7000, queued: null },
      { id: "c", parent: "a", label: "Tool", kind: "tool", start: 4000, end: null, queued: null },
    ]);
    expect(layout.origin).toBe(1000);
    expect(layout.nodes.find(x => x.id === "a")!.lane).toBeGreaterThan(0);
    expect(layout.nodes.find(x => x.id === "a")!.lane).not.toBe(layout.nodes.find(x => x.id === "b")!.lane);
    expect(layout.nodes.find(x => x.id === "c")!.lane).toBeGreaterThan(layout.nodes.find(x => x.id === "a")!.lane);
    expect(layout.nodes.find(x => x.id === "a")!.queued).toBe(2000);
    expect(layout.nodes.find(x => x.id === "c")!.end).toBeNull();
  });
  it("renders real SVG source-time spans and reports exact selected identities", () => {
    const host = document.createElement("div"), chosen: string[] = [];
    renderChart(host, { id: "work-waterfall", type: "waterfall", spans: [
      { id: "source-id", parent: null, label: "<img src=x onerror=alert(1)>", kind: "test", start: "2026-11-01T00:00:00Z", end: "2026-11-01T00:00:30Z", queued: null },
    ] }, event => chosen.push(event.id));
    expect(host.querySelectorAll("svg rect")).toHaveLength(1);
    expect(host.querySelector("rect")!.getAttribute("height")).toBe("480");
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toContain("<img src=x onerror=aler");
    host.querySelector<SVGGElement>("[role=button]")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(chosen).toEqual(["source-id"]);
  });
  it("retains detached parentage and cyclic evidence without inventing a source boundary", () => {
    const result = layoutWaterfall([
      { id: "detached", parent: "not-retained", label: "Detached", kind: "custom", start: null, end: null, queued: null },
      { id: "a", parent: "b", label: "A", kind: "tool", start: 1, end: 2, queued: null },
      { id: "b", parent: "a", label: "B", kind: "tool", start: 1, end: 2, queued: null },
    ]);
    expect(result.nodes).toHaveLength(3);
    expect(result.nodes.find(node => node.id === "detached")).toMatchObject({ unresolved: true, start: null, end: null });
  });
  it("ends queue-only canceled tasks at their known finish rather than the session horizon", () => {
    const host = document.createElement("div");
    renderChart(host, { id: "queue", type: "waterfall", spans: [
      { id: "session", parent: null, label: "Session", kind: "session", start: 0, end: 100000, queued: null },
      { id: "queued", parent: "session", label: "Canceled queued task", kind: "test", start: null, end: 30000, queued: 10000 },
    ] }, () => {});
    const task = [...host.querySelectorAll<SVGGElement>("[role=button]")].find(group => group.textContent?.includes("Canceled queued task"))!;
    expect(task.querySelector("line")!.getAttribute("y2")).toBe("179");
  });

it("shows a single date for one observed timestamp rather than repeating an artificial range", () => {
  const host = document.createElement("div");
  renderChart(host, { id: "single", type: "trend", points: [
    { id: "run", label: "Recorded run", at: "2026-11-01T00:00:00Z", value: 12 },
  ] }, () => {});
  expect(host.querySelectorAll(".chart-axis:not(.grid) .tick")).toHaveLength(1);
  expect(host.querySelector(".chart-axis:not(.grid)")!.textContent).toContain("Nov 01");
});

it("labels each day once across a multi-day trend instead of repeating dates", () => {
  const host = document.createElement("div");
  Object.defineProperty(host, "clientWidth", { value: 1400 });
  renderChart(host, { id: "days", type: "trend", points: Array.from({ length: 4 }, (_, day) => (
    { id: `day-${day}`, label: `Day ${day}`, at: new Date(Date.UTC(2026, 9, 29 + day)).toISOString(), value: day }
  )) }, () => {});
  const labels = [...host.querySelectorAll(".chart-axis:not(.grid) .tick text")].map(label => label.textContent);
  expect(labels.length).toBeGreaterThan(1);
  expect(new Set(labels).size).toBe(labels.length);
});

it("draws separate eval observations as dots without a connecting line", () => {
  const host = document.createElement("div");
  renderChart(host, { id: "metric", type: "trend", line: false, points: [
    { id: "a", label: "a", at: "2026-11-01T00:00:00Z", value: 1 },
    { id: "b", label: "b", at: "2026-11-02T00:00:00Z", value: 2 },
  ] }, () => {});
  expect(host.querySelectorAll("circle")).toHaveLength(2);
  expect(host.querySelector("path[stroke-width='2']")).toBeNull();
});

it("draws one column per non-empty time bin with its exact count", () => {
  const host = document.createElement("div");
  renderChart(host, { id: "commands", type: "histogram", binSeconds: 21600, bins: [
    { start: "2026-11-01T00:00:00Z", count: 2 },
    { start: "2026-11-01T06:00:00Z", count: 0 },
    { start: "2026-11-01T12:00:00Z", count: 5 },
  ] }, () => {});
  expect(host.querySelectorAll("rect")).toHaveLength(2);
  expect([...host.querySelectorAll("rect title")].map(title => title.textContent)).toEqual(["2026-11-01 00:00 UTC: 2", "2026-11-01 12:00 UTC: 5"]);
});
