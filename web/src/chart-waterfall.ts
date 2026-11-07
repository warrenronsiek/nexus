// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import type { Chart, Choose, Span } from "./chart-model";
import { actionable, attrs, element, type Svg, type Group } from "./chart-svg";
import { layoutFlow, labelBox, type Box, type FlowNode } from "./flow-layout";
import { flowLink } from "./flow-links";
import { connected, returnTime, segments, type FlowSegment } from "./flow-state";

function color(kind: string): string {
  const family = kind.split(".")[0];
  return `var(--${({ host: "agent", agent: "agent", subagent: "agent", test: "test", script: "script", review: "review", tool: "tool", command: "tool" } as Record<string, string>)[family] ?? "session"})`;
}
function duration(span: Span): string {
  if (span.start === null || span.end === null) return "duration unknown";
  if (span.end < span.start) return "conflicting boundaries";
  const seconds = (span.end - span.start) / 1000;
  return seconds >= 3600 ? `${d3.format(".2~f")(seconds / 3600)}h` : seconds >= 60 ? `${d3.format(".1~f")(seconds / 60)}m` : `${d3.format(".2~f")(seconds)}s`;
}
function qualification(node: FlowNode): string {
  if (node.span.conflicts?.length) return " · conflicting evidence";
  if (node.span.parent && !node.parent) return " · parent unresolved";
  return node.span.callMode === "asynchronous" ? " · async" : "";
}
function geometry(chart: Chart) {
  const times = (chart.spans ?? []).flatMap(node => [node.queued, node.start, node.end, node.returnedAt]).filter((value): value is number => value != null && Number.isFinite(value));
  const origin = times.length ? Math.min(...times) : 0, horizon = times.length ? Math.max(...times) : 0;
  const height = Math.max(360, 1000 * (chart.zoom ?? 1));
  const y = d3.scaleLinear().domain([origin, Math.max(origin + 1, horizon)]).range([116, height - 48]);
  return { origin, horizon, height, y, timed: times.length > 0 };
}
/** D3 owns drawing and collision layout; Elm retains state and exact source evidence. */
export function drawWaterfall(svg: Svg, chart: Chart, width: number, choose: Choose): void {
  new ExecutionFlow(svg, chart, width, choose).draw();
}
class ExecutionFlow {
  private projection;
  private nodes: FlowNode[];
  private fullWidth: number;
  private arrow: string;
  private labelBoxes: Box[];
  constructor(private svg: Svg, private chart: Chart, width: number, private choose: Choose) {
    this.projection = geometry(chart);
    this.nodes = layoutFlow(chart.spans ?? [], this.projection.y, this.projection.horizon);
    this.fullWidth = Math.max(width, 900, ...this.nodes.map(node => node.x + node.labelWidth + 30));
    this.labelBoxes = this.nodes.filter(node => node.span.kind !== "work.session").map(labelBox);
    this.arrow = `${chart.id}-return-arrow`;
  }
  draw(): void {
    const { height } = this.projection;
    attrs(this.svg, { viewBox: `0 0 ${this.fullWidth} ${height}`, width: this.fullWidth, height });
    this.guides();
    const edges = this.svg.append("g").attr("class", "flow-edges");
    for (const node of this.nodes) {
      const at = node.span.queued ?? node.span.start;
      if (at !== null && connected(node, at) && !node.span.conflicts?.length) this.connector(edges, node, at, "dispatch");
      const back = returnTime(node.span);
      if (back !== null && connected(node, back)) this.connector(edges, node, back, "return");
    }
    for (const node of this.nodes) this.drawNode(node);
  }
  private guides(): void {
    const { origin, height, y, timed } = this.projection;
    this.svg.append("defs").append("marker").attr("id", this.arrow).attr("viewBox", "0 0 10 10").attr("refX", 8).attr("refY", 5).attr("markerWidth", 6).attr("markerHeight", 6).attr("orient", "auto-start-reverse").append("path").attr("d", "M0,1 L9,5 L0,9").attr("fill", "var(--muted)");
    element(this.svg, "text", { x: 20, y: 24, class: "chart-label" }, "Execution flow · time flows down");
    element(this.svg, "text", { x: 20, y: 44, class: "chart-label flow-origin" }, timed ? `UTC origin ${new Date(origin).toISOString()}` : "Source boundaries unavailable");
    this.svg.append("g").attr("class", "chart-axis grid").attr("transform", "translate(74,0)").call(d3.axisLeft(y).ticks(Math.max(4, height / 150)).tickFormat(time => `+${d3.format(".3~f")((Number(time) - origin) / 1000)}s`).tickSize(-(this.fullWidth - 90)));
  }
  private connector(group: Group, node: FlowNode, at: number, kind: "dispatch" | "return"): void {
    const parent = node.parent!.x + 6, child = node.x - 6, y = this.projection.y(at);
    const [from, to] = kind === "return" ? [child, parent] : [parent, child];
    element(group, "path", { class: `flow-${kind}`, "data-at": at, "data-owner": node.span.id, d: flowLink(from, to, y, this.labelBoxes), "stroke-linejoin": "round", stroke: "var(--muted)", "stroke-width": 1.2, fill: "none", "marker-end": `url(#${this.arrow})` });
  }
  private drawNode(node: FlowNode): void {
    const { span, x } = node;
    const group = this.svg.append("g").attr("class", `waterfall-node${span.id === this.chart.selected ? " selected" : ""}`).attr("data-node-id", span.id).attr("data-x", x);

    this.intervals(group, node);
    this.label(group, node);
    const detail = `${span.label} · ${span.kind} · ${duration(span)}${qualification(node)} · ${span.role ?? "unknown"}`;
    element(group, "title", {}, `${detail}${span.end === null ? " · finish unknown" : ""}`);
    group.attr("aria-label", detail);
    actionable(group, span.id, this.choose);
  }
  private intervals(group: Group, node: FlowNode): void {
    const { span, x } = node, { y, horizon } = this.projection;
    if (span.queued !== null) this.stripe(group, node, { start: span.queued, end: span.start ?? span.end ?? horizon, state: "wait" }, "flow-queue");
    for (const segment of segments(node, this.nodes, horizon)) this.stripe(group, node, segment);
    if (span.end !== null && !span.conflicts?.length) element(group, "line", { class: "flow-end", "data-at": span.end, x1: x - 9, x2: x + 9, y1: y(span.end), y2: y(span.end), stroke: color(span.kind), "stroke-width": 3 });
    if (span.start === null && span.queued === null && span.end !== null) element(group, "circle", { cx: x, cy: y(span.end), r: 5, fill: "none", stroke: color(span.kind) });
  }
  private label(group: Group, node: FlowNode): void {
    const { span, x, labelY } = node;
    if (span.kind === "work.session") return;
    const peers = this.nodes.filter(other => other.span.label === span.label);
    const sourceLabel = peers.length > 1 ? `${span.label} · ${peers.indexOf(node) + 1}` : span.label;
    const label = sourceLabel.length > 26 ? `${sourceLabel.slice(0, 25)}…` : sourceLabel;
    const anchor = span.start ?? span.queued ?? span.end;
    if (anchor !== null && Math.abs(labelY - this.projection.y(anchor)) > 18) {
      element(group, "path", { class: "flow-label-leader", d: `M${x + 7},${this.projection.y(anchor)}L${x + 16},${labelY - 4}`, stroke: "var(--muted)", fill: "none", "stroke-width": .8 });
    }
    element(group, "rect", { class: "flow-label-ground", x: x + 13, y: labelY - 14, width: node.labelWidth, height: 38, rx: 2, fill: "var(--ground)" });
    element(group, "text", { class: "chart-label flow-label", x: x + 20, y: labelY }, label);
    element(group, "text", { class: "chart-label flow-meta", x: x + 20, y: labelY + 17 }, `${duration(span)}${qualification(node)}`);
    if (span.start === null && span.queued === null && span.end === null) element(group, "text", { class: "chart-label", x, y: labelY + 46 }, "Source boundaries unavailable");
  }
  private stripe(group: Group, node: FlowNode, segment: FlowSegment, extra = ""): void {
    const { start, end, state } = segment, ink = color(node.span.kind), y = this.projection.y;
    if (end < start) return;
    element(group, "rect", { class: `flow-${state} ${extra}`, "data-start": start, "data-end": end, x: node.x - 6, y: y(start), width: 12, height: Math.max(2, y(end) - y(start)), rx: 1, fill: state === "active" ? ink : "none", stroke: state === "unknown" ? "var(--muted)" : ink, "stroke-width": 1.3, "stroke-dasharray": state === "wait" ? "2 4" : state === "unknown" ? "6 4" : null, "stroke-opacity": state === "unknown" ? .55 : 1 });
  }
}
