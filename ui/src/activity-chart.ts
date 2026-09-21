// @feature observability-ui
// @spec docs/features/observability-ui.md
// @entrypoint renderActivityChart
// @boundary d3-dom
import { axisBottom, axisLeft, scaleBand, scaleLinear, select } from "d3";

export type ActivityClass = "activity" | "info" | "warning" | "critical";

export interface ActivitySeries {
  class: ActivityClass;
  count: number;
}

export interface ActivityBucket {
  startMilliseconds: number;
  series: ActivitySeries[];
}

interface StackedDatum {
  bucket: number;
  class: ActivityClass;
  lower: number;
  upper: number;
}

const colors: Record<ActivityClass, string> = {
  activity: "#65758b",
  info: "#2f81f7",
  warning: "#d29922",
  critical: "#f85149",
};

export function renderActivityChart(buckets: ActivityBucket[]): void {
  const container = document.querySelector<HTMLElement>("#activity-chart");
  if (container === null) {
    return;
  }
  select(container).selectAll(".chart-empty").remove();
  if (buckets.length === 0) {
    select(container).selectAll("svg").remove();
    select(container)
      .append("p")
      .attr("class", "chart-empty")
      .attr("role", "status")
      .text("No activity in the last hour");
    return;
  }

  const width = Math.max(container.clientWidth, 640);
  const height = 240;
  const margin = { top: 12, right: 16, bottom: 38, left: 38 };
  const plotWidth = width - margin.left - margin.right;
  const plotHeight = height - margin.top - margin.bottom;
  const stacked = stackBuckets(buckets);
  const totals = buckets.map((bucket) =>
    bucket.series.reduce((sum, item) => sum + item.count, 0),
  );
  const x = scaleBand<number>()
    .domain(buckets.map((bucket) => bucket.startMilliseconds))
    .range([0, plotWidth])
    .padding(0.18);
  const y = scaleLinear()
    .domain([0, Math.max(...totals, 1)])
    .nice()
    .range([plotHeight, 0]);
  const svg = select(container)
    .selectAll<SVGSVGElement, null>("svg")
    .data([null])
    .join("svg")
    .attr("role", "img")
    .attr("aria-label", "Nexus activity over the last hour")
    .attr("viewBox", `0 0 ${width} ${height}`);
  svg.selectAll("*").remove();
  const plot = svg
    .append("g")
    .attr("transform", `translate(${margin.left},${margin.top})`);
  plot
    .selectAll("rect")
    .data(stacked)
    .join("rect")
    .attr("data-series", (datum) => datum.class)
    .attr("x", (datum) => x(datum.bucket) ?? 0)
    .attr("width", x.bandwidth())
    .attr("y", (datum) => y(datum.upper))
    .attr("height", (datum) => y(datum.lower) - y(datum.upper))
    .attr("fill", (datum) => colors[datum.class]);
  plot
    .append("g")
    .attr("class", "chart-axis")
    .attr("transform", `translate(0,${plotHeight})`)
    .call(
      axisBottom(x)
        .tickValues(tickValues(buckets))
        .tickFormat((milliseconds) =>
          new Date(milliseconds).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          }),
        ),
    );
  plot
    .append("g")
    .attr("class", "chart-axis")
    .call(axisLeft(y).ticks(4).tickFormat((value) => String(value)));
}

function stackBuckets(buckets: ActivityBucket[]): StackedDatum[] {
  return buckets.flatMap((bucket) => {
    let lower = 0;
    return bucket.series.map((item) => {
      const datum = {
        bucket: bucket.startMilliseconds,
        class: item.class,
        lower,
        upper: lower + item.count,
      };
      lower = datum.upper;
      return datum;
    });
  });
}

function tickValues(buckets: ActivityBucket[]): number[] {
  const step = Math.max(Math.ceil(buckets.length / 6), 1);
  return buckets
    .filter((_, index) => index % step === 0)
    .map((bucket) => bucket.startMilliseconds);
}
