// @feature observability-ui
// @spec docs/features/observability-ui.md
import { visibleWidth } from "@earendil-works/pi-tui";
import { expect, it } from "vitest";
import { chartBars, chartTrend, formatDurationSeconds } from "../src/charts.ts";

it("compares measured values without drawing unknowns as zero", () => {
  const lines = chartBars([
    { label: "tests", value: 20, valueLabel: "20s · 4 attempts" },
    { label: "reviews", value: 10, valueLabel: "10s · 2 attempts" },
    { label: "waiting", value: null, valueLabel: "unknown" },
  ], 70);
  expect(lines[0]).toContain("20s · 4 attempts");
  expect((lines[0].match(/█/gu) ?? []).length).toBeGreaterThan((lines[1].match(/█/gu) ?? []).length);
  expect(lines[2]).toContain("?");
  expect(lines[2]).toContain("unknown");
  expect(lines[2]).not.toContain("█");
});

it("shows a labeled trend with missing observations and signed values", () => {
  const lines = chartTrend([
    { label: "Oct 01", value: -5 }, { label: "Oct 02", value: null }, { label: "Oct 03", value: 5 },
  ], 48, String);
  const rendered = lines.join("\n");
  expect(rendered).toContain("-5");
  expect(rendered).toContain("5");
  expect(rendered).toContain("Oct 01");
  expect(rendered).toContain("Oct 03");
  expect(rendered).toContain("?");
  expect(rendered).toContain("●");
  expect(chartTrend([{ label: "unknown", value: null }], 48, String).join("\n")).toContain("No numeric observations");
});

it.each([1, 8, 24, 80])("fits width %i and wide untrusted labels without executing terminal controls", width => {
    const rows = [{ label: "界界\u001b]52;c;payload\u0007", value: 1, valueLabel: "1s" }];
    const points = rows.map(row => ({ label: row.label, value: row.value }));
    for (const line of [...chartBars(rows, width), ...chartTrend(points, width, String)]) {
      expect(visibleWidth(line)).toBeLessThanOrEqual(width);
      expect(line).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/u);
    }
});

it("qualifies bounded chart windows and formats durations at human scale", () => {
  const lines = chartTrend(Array.from({ length: 40 }, (_, index) => ({ label: `day ${index}`, value: index })), 30, String);
  expect(lines.join("\n")).toContain("Latest");
  expect(lines.join("\n")).toContain("of 40");
  expect(formatDurationSeconds(0.25)).toBe("250ms");
  expect(formatDurationSeconds(90)).toBe("1.5m");
  expect(formatDurationSeconds(7200)).toBe("2h");
});

it("positions dated observations proportionally across the time axis", () => {
  const lines = chartTrend([
    { label: "start", value: 1, at: 0 }, { label: "near start", value: 1, at: 1 }, { label: "end", value: 1, at: 100 },
  ], 50, String);
  const row = lines.find(line => line.includes("●"))!;
  expect(row.lastIndexOf("●") - row.indexOf("●")).toBeGreaterThan(30);
  expect(lines.join("\n")).toContain("Time axis");
});
