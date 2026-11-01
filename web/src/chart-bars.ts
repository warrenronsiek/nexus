// @feature observability-ui
// @spec docs/features/observability-ui.md
import * as d3 from "d3";
import { actionable, attrs, element, type Svg } from "./chart-svg";
import type { Bar, Choose } from "./chart-model";

export function drawBars(svg: Svg, data: Bar[], width: number, choose: Choose): void {
  const height = Math.max(100, data.length * 48 + 24), left = Math.min(240, width * .3);
  attrs(svg, { viewBox: `0 0 ${width} ${height}`, height });
  const scale = d3.scaleLinear().domain([0, d3.max(data, item => item.value ?? 0) || 1]).range([0, width - left - 100]);
  data.forEach((item, index) => drawBar(svg, item, { index, left, scale }, choose));
  if (!data.length) element(svg, "text", { x: 20, y: 50, class: "chart-label" }, "No recorded observations in this scope.");
}
interface BarPosition { index: number; left: number; scale: d3.ScaleLinear<number, number> }
function drawBar(svg: Svg, item: Bar, position: BarPosition, choose: Choose): void {
  const { index, left, scale } = position, row = svg.append("g").attr("transform", `translate(0,${index * 48 + 10})`);
  const label = item.label.length > 29 ? `${item.label.slice(0, 28)}…` : item.label;
  element(row, "text", { x: 0, y: 17, class: "chart-label" }, label);
  if (item.value !== null) element(row, "rect", { x: left, height: 25, width: Math.max(2, scale(item.value)), rx: 5, fill: index % 2 ? "#638ff3" : "#5edbb6" });
  const display = item.detail || (item.value === null ? "Unknown" : d3.format(",.0f")(item.value));
  element(row, "text", { x: left + scale(item.value ?? 0) + 12, y: 17, class: "chart-value" }, display);
  element(row, "title", {}, `${item.label}: ${display}`);
  actionable(row, item.id, choose);
}
