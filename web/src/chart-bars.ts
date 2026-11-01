// @feature observability-ui
// @spec docs/features/observability-ui.md
import * as d3 from "d3";
import { actionable, attrs, element, type Svg } from "./chart-svg";
import type { Bar, Choose } from "./chart-model";

export function drawBars(svg: Svg, data: Bar[], width: number, choose: Choose): void {
  const height = Math.max(100, data.length * 48 + 12);
  attrs(svg, { viewBox: `0 0 ${width} ${height}`, height });
  const scale = d3.scaleLinear().domain([0, d3.max(data, item => item.value ?? 0) || 1]).range([0, width - 8]);
  data.forEach((item, index) => drawBar(svg, item, { index, width, scale }, choose));
  if (!data.length) element(svg, "text", { x: 8, y: 50, class: "chart-label" }, "No recorded observations in this scope.");
}
interface BarPosition { index: number; width: number; scale: d3.ScaleLinear<number, number> }
function drawBar(svg: Svg, item: Bar, position: BarPosition, choose: Choose): void {
  const { index, width, scale } = position, row = svg.append("g").attr("transform", `translate(0,${index * 48 + 4})`);
  const labelBudget = Math.max(8, Math.floor(width * .42 / 7));
  const label = item.label.length > labelBudget ? `${item.label.slice(0, labelBudget - 1)}…` : item.label;
  element(row, "text", { x: 0, y: 14, class: "chart-label" }, label);
  if (item.value !== null) element(row, "rect", { x: 0, y: 25, height: 8, width: Math.max(2, scale(item.value)), rx: 1, fill: "var(--chart-ink, #73a9ec)" });
  const display = item.detail || (item.value === null ? "Unknown" : d3.format(",.0f")(item.value));
  element(row, "text", { x: width - 8, y: 14, "text-anchor": "end", class: "chart-value" }, display);
  element(row, "title", {}, `${item.label}: ${display}`);
  actionable(row, item.id, choose);
}
