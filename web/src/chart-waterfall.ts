// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import { actionable, attrs, element, type Group, type Svg } from "./chart-svg";
import { layoutWaterfall } from "./waterfall-layout";
import type { Chart, Choose, Placed } from "./chart-model";

const COLORS: Record<string, string> = { agent: "#a797ff", subagent: "#a797ff", test: "#5edbb6", review: "#f697aa", script: "#eac47e", tool: "#75c6f3", command: "#75c6f3", session: "#8295ae" };
function color(kind: string): string {
  const family = kind === "host.session" || kind === "work.session" ? "session" : kind.toLowerCase().split(".")[0];
  return COLORS[family] ?? "#8295ae";
}
interface Geometry { x: number; start: number | null; end: number | null; queued: number | null; bottom: number; ink: string; parentX: number | null; selected: boolean }
interface Projection { y: d3.ScaleLinear<number, number>; bottom: number; selected: string | null }
function geometry(node: Placed, projection: Projection): Geometry {
  const position = (time: number | null) => time === null ? null : projection.y(time);
  return { x: 90 + node.lane * 180, start: position(node.start), end: position(node.end), queued: position(node.queued), bottom: projection.bottom, ink: color(node.kind), parentX: node.parentLane === null ? null : 98 + node.parentLane * 180, selected: node.id === projection.selected };
}
function branch(group: Group, shape: Geometry): void {
  const at = shape.queued ?? shape.start;
  if (shape.parentX !== null && at !== null) element(group, "path", { d: `M${shape.parentX},${at}H${shape.x}`, stroke: shape.ink, fill: "none", "stroke-opacity": .6 });
}
function queueSpan(group: Group, shape: Geometry): void {
  if (shape.queued !== null) element(group, "line", { x1: shape.x + 8, x2: shape.x + 8, y1: shape.queued, y2: shape.start ?? shape.end ?? shape.bottom, stroke: shape.ink, "stroke-width": 3, "stroke-dasharray": "3 5" });
}
function executionSpan(group: Group, shape: Geometry): void {
  if (shape.start === null) return;
  element(group, "rect", { x: shape.x, y: shape.start, width: 16, height: Math.max(5, (shape.end ?? shape.bottom) - shape.start), rx: 5, fill: shape.ink, "fill-opacity": shape.selected ? .95 : .35, stroke: shape.ink, "stroke-width": shape.selected ? 2 : 1, "stroke-dasharray": shape.end === null ? "4 4" : null });
}
function label(group: Group, node: Placed, shape: Geometry): void {
  const at = shape.queued ?? shape.start ?? shape.end;
  if (at === null) {
    element(group, "text", { x: shape.x, y: shape.bottom + 25, class: "chart-label" }, `${node.label.slice(0, 18)} · no boundaries`);
  } else {
    element(group, "circle", { cx: shape.x + 8, cy: at, r: 5, fill: shape.ink });
    element(group, "text", { x: shape.x + 27, y: at + 16, class: "chart-label" }, node.label.slice(0, 23));
  }
  element(group, "title", {}, `${node.label} · ${node.kind}${node.end === null ? " · finish unknown" : ""}${node.unresolved ? " · parent unresolved" : ""}`);
}
function drawNode(svg: Svg, node: Placed, projection: Projection, choose: Choose): void {
  const shape = geometry(node, projection), group = svg.append("g").attr("class", shape.selected ? "waterfall-node selected" : "waterfall-node");
  branch(group, shape); queueSpan(group, shape); executionSpan(group, shape); label(group, node, shape);
  actionable(group, node.id, choose);
}
export function drawWaterfall(svg: Svg, chart: Chart, width: number, choose: Choose): void {
  const layout = layoutWaterfall(chart.spans ?? []), fullWidth = Math.max(width, 270 + layout.lanes * 180);
  const height = Math.max(420, Math.min(16000, 560 * (chart.zoom ?? 1))), bottom = height - 45;
  attrs(svg, { viewBox: `0 0 ${fullWidth} ${height}`, width: fullWidth, height });
  const y = d3.scaleLinear().domain([layout.origin, Math.max(layout.origin + 1, layout.end)]).range([35, bottom]);
  const hasTime = layout.nodes.some(node => node.start !== null || node.end !== null || node.queued !== null);
  element(svg, "text", { x: 90, y: 17, class: "chart-label" }, hasTime ? `UTC origin ${new Date(layout.origin).toISOString()}` : "Source boundaries unavailable");
  svg.append("g").attr("class", "chart-axis grid").attr("transform", "translate(66,0)").call(d3.axisLeft(y).ticks(Math.max(5, height / 80)).tickFormat(time => `+${d3.format(".2~f")((Number(time) - layout.origin) / 1000)}s`).tickSize(-fullWidth));
  layout.nodes.forEach(node => drawNode(svg, node, { y, bottom, selected: chart.selected ?? null }, choose));
}
