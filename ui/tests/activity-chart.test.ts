// @feature observability-ui
// @spec docs/features/observability-ui.md
// @boundary jsdom
/** @vitest-environment jsdom */
import { beforeEach, describe, expect, test } from "vitest";
import { renderActivityChart, type ActivityBucket } from "../src/activity-chart";

const buckets: ActivityBucket[] = [
  {
    startMilliseconds: 3_300_000,
    series: [
      { class: "activity", count: 4 },
      { class: "critical", count: 1 },
    ],
  },
];

describe("activity chart", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="activity-chart"></div>';
  });

  test("renders typed severity series into one reusable SVG", () => {
    renderActivityChart(buckets);
    renderActivityChart(buckets);

    expect(document.querySelectorAll("#activity-chart svg")).toHaveLength(1);
    expect(document.querySelectorAll("#activity-chart rect[data-series]")).toHaveLength(2);
    expect(document.querySelector("svg")?.getAttribute("aria-label")).toContain(
      "activity over the last hour",
    );
  });

  test("renders an accessible empty state", () => {
    renderActivityChart([]);

    expect(document.querySelector("#activity-chart")?.textContent).toContain(
      "No activity in the last hour",
    );
  });

  test("updates its responsive view box when the container is resized", () => {
    const container = document.querySelector<HTMLElement>("#activity-chart");
    Object.defineProperty(container, "clientWidth", { value: 720, configurable: true });
    renderActivityChart(buckets);
    expect(document.querySelector("svg")?.getAttribute("viewBox")).toBe("0 0 720 240");

    Object.defineProperty(container, "clientWidth", { value: 960, configurable: true });
    renderActivityChart(buckets);
    expect(document.querySelector("svg")?.getAttribute("viewBox")).toBe("0 0 960 240");
    expect(document.querySelectorAll("#activity-chart svg")).toHaveLength(1);
  });
});
