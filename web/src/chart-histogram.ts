// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import * as d3 from "d3";
import { attrs, element, type Svg } from "./chart-svg";
import type { Bin } from "./chart-model";

interface Frame { x: d3.ScaleTime<number, number>; y: d3.ScaleLinear<number, number>; bottom: number; span: number }
const HEIGHT = 78, BOTTOM = 56;

/** Fixed time bins as columns: one compact histogram per command. */
export function drawHistogram(svg: Svg, bins: Bin[], binSeconds: number, width: number): void {
  attrs(svg, { viewBox: `0 0 ${width} ${HEIGHT}`, height: HEIGHT });
  const dated = bins.filter(bin => Number.isFinite(bin.start)).sort((a, b) => a.start - b.start);
  if (!dated.length) { element(svg, "text", { x: 4, y: 30, class: "chart-label" }, "No runs in this window."); return; }
  const span = binSeconds * 1000, peak = d3.max(dated, bin => bin.count) || 1;
  const x = d3.scaleUtc().domain([dated[0].start, dated[dated.length - 1].start + span]).range([2, width - 34]);
  const frame = { x, y: d3.scaleLinear().domain([0, peak]).range([BOTTOM, 6]), bottom: BOTTOM, span };
  dated.forEach(bin => drawBin(svg, bin, frame));
  element(svg, "text", { x: width - 30, y: 14, class: "chart-value" }, d3.format(",")(peak));
  const days = Math.max(1, Math.ceil((frame.x.domain()[1].getTime() - dated[0].start) / 86400000 / Math.max(2, Math.floor(width / 70))));
  const axis = d3.axisBottom(x).ticks(d3.utcDay.every(days)!).tickFormat(d3.utcFormat("%b %d") as (value: Date | d3.NumberValue, index: number) => string).tickSizeOuter(0);
  svg.append("g").attr("class", "chart-axis").attr("transform", `translate(0,${BOTTOM})`).call(axis);
}
function drawBin(svg: Svg, bin: Bin, { x, y, bottom, span }: Frame): void {
  if (bin.count <= 0) return;
  const left = x(bin.start), column = svg.append("rect");
  attrs(column, { x: left, y: y(bin.count), width: Math.max(1, x(bin.start + span) - left - 1), height: Math.max(1, bottom - y(bin.count)), fill: "var(--chart-ink, #73a9ec)" });
  element(column, "title", {}, `${new Date(bin.start).toISOString().slice(0, 16).replace("T", " ")} UTC: ${bin.count}`);
}
