// @vitest-environment jsdom
// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import { expect, it } from "vitest";
import { renderChart } from "../src/charts";

  it("renders real SVG source-time spans and reports exact selected identities", () => {
    const host = document.createElement("div"), chosen: string[] = [];
    renderChart(host, { id: "work-waterfall", type: "waterfall", spans: [
      { id: "source-id", parent: null, label: "<img src=x onerror=alert(1)>", kind: "test", start: "2026-11-01T00:00:00Z", end: "2026-11-01T00:00:30Z", queued: null },
    ] }, event => chosen.push(event.id));
    expect(host.querySelectorAll(".waterfall-node")).toHaveLength(1);
    expect(host.querySelector(".flow-unknown")!.getAttribute("width")).toBe("12");
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toContain("<img src=x onerror=aler");
    host.querySelector<SVGGElement>("[role=button]")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(chosen).toEqual(["source-id"]);
  });
  it("ends queue-only canceled tasks at their known finish rather than the session horizon", () => {
    const host = document.createElement("div");
    renderChart(host, { id: "queue", type: "waterfall", spans: [
      { id: "session", parent: null, label: "Session", kind: "session", start: 0, end: 100000, queued: null },
      { id: "queued", parent: "session", label: "Canceled queued task", kind: "test", start: null, end: 30000, queued: 10000 },
    ] }, () => {});
    const task = [...host.querySelectorAll<SVGGElement>("[role=button]")].find(group => group.textContent?.includes("Canceled queued task"))!;
    expect(task.querySelector(".flow-queue")!.getAttribute("data-end")).toBe("30000");
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
