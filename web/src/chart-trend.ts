// @feature observability-ui
// @feature evals
// @spec docs/features/observability-ui.md
// @spec docs/features/evals.md
import * as d3 from "d3";
import { actionable, attrs, element, type Svg } from "./chart-svg";
import type { Choose, Point } from "./chart-model";

interface Axes { x: d3.ScaleTime<number, number>; y: d3.ScaleLinear<number, number>; left: number; bottom: number }
function axes(data: Point[], width: number): Axes {
  const extent = d3.extent(data, point => point.at), min = extent[0] ?? Date.now(), max = extent[1] ?? min + 86400000;
  const x = d3.scaleUtc().domain([min, max === min ? max + 3600000 : max]).range([68, width - 32]);
  const values = data.flatMap(point => point.value === null ? [] : [point.value]);
  const lo = Math.min(0, d3.min(values) ?? 0), hi = d3.max(values) ?? 1;
  const y = d3.scaleLinear().domain([lo, hi === lo ? hi + 1 : hi]).nice().range([216, 20]);
  return { x, y, left: 68, bottom: 216 };
}
function drawAxes(svg: Svg, { x, y, left, bottom }: Axes, width: number): void {
  svg.append("g").attr("class", "chart-axis grid").attr("transform", `translate(${left},0)`).call(d3.axisLeft(y).ticks(5).tickSize(left - width + 32));
  const format = d3.utcFormat("%b %d") as (value: Date | d3.NumberValue, index: number) => string;
  svg.append("g").attr("class", "chart-axis").attr("transform", `translate(0,${bottom})`).call(d3.axisBottom(x).ticks(Math.max(2, Math.floor(width / 150))).tickFormat(format));
}
function drawPoint(svg: Svg, point: Point, { x, y, bottom }: Axes, choose: Choose): void {
  const mark = svg.append("circle");
  attrs(mark, { cx: x(point.at), cy: point.value === null ? bottom : y(point.value), r: point.value === null ? 4 : 5, fill: point.value === null ? "#172539" : "#5edbb6", stroke: "#5edbb6" });
  element(mark, "title", {}, `${point.label}: ${point.value ?? "unavailable"}`);
  actionable(mark, point.id, choose);
}
export function drawTrend(svg: Svg, data: Point[], width: number, choose: Choose): void {
  attrs(svg, { viewBox: `0 0 ${width} 260`, height: 260 });
  const dated = data.filter(point => Number.isFinite(point.at)), scales = axes(dated, width);
  drawAxes(svg, scales, width);
  const line = d3.line<Point>().defined(point => point.value !== null).x(point => scales.x(point.at)).y(point => scales.y(point.value!));
  svg.append("path").datum(dated).attr("d", line).attr("fill", "none").attr("stroke", "#5edbb6").attr("stroke-width", 2.5);
  dated.forEach(point => drawPoint(svg, point, scales, choose));
  if (!dated.some(point => point.value !== null)) element(svg, "text", { x: scales.left + 16, y: 100, class: "chart-label" }, "No numeric observations; inspect exact recorded values below.");
}
