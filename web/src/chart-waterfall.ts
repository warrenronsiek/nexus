// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import type { Chart, Choose, Span } from "./chart-model";
import { actionable, attrs, element, type Svg, type Group } from "./chart-svg";
import { dispatchTime, layoutFlow, labelBox, type Box, type FlowNode } from "./flow-layout";
import { flowLink } from "./flow-links";
import { flowTime } from "./flow-time";
import { drawFlowGuides } from "./flow-axis";
import { connected, returnTime, segments, type FlowSegment } from "./flow-state";

function color(kind: string): string {
  const family = kind.split(".")[0];
  return `var(--${({ host: "agent", agent: "agent", subagent: "agent", test: "test", script: "script", review: "review", tool: "tool", command: "tool" } as Record<string, string>)[family] ?? "session"})`;
}
function duration(span: Span): string {
  if (span.start === null) return "start unobserved";
  if (span.end === null) return "finish unobserved";
  if (span.end < span.start) return "conflicting boundaries";
  const seconds = span.elapsed ? Number(span.elapsed.nanoseconds) / 1e9 : (span.end - span.start) / 1000;
  if (span.executionQualified !== true || !Number.isFinite(seconds) || seconds < 0) return "duration unqualified";
  return seconds >= 3600 ? `${d3.format(".2~f")(seconds / 3600)}h` : seconds >= 60 ? `${d3.format(".1~f")(seconds / 60)}m` : `${d3.format(".2~f")(seconds)}s`;
}
function activityBucket(span: Span): string {
  return [...new Set((span.operations ?? []).map(operation => operation.category_name))].join(", ");
}
function qualification(node: FlowNode): string {
  if (node.span.conflicts?.length) return " · conflicting evidence";
  if (node.span.parent && !node.parent) return " · parent unresolved";
  return node.span.callMode === "asynchronous" ? " · async" : "";
}
function truncateLabel(label: string): string {
  return label.length > 26 ? `${label.slice(0, 25)}…` : label;
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
  private clip: string | null;
  constructor(private svg: Svg, private chart: Chart, width: number, private choose: Choose) {
    this.projection = flowTime(chart.spans ?? [], chart.zoom ?? 1);
    const { y, horizon, cuts } = this.projection;
    const breaks = cuts.map(({ start, end }) => ({ start: y(start), end: y(end) }));
    this.nodes = layoutFlow(chart.spans ?? [], { y, horizon, breaks });
    this.fullWidth = Math.max(width, 900, ...this.nodes.map(node => node.x + node.labelWidth + 30));
    this.labelBoxes = this.nodes.filter(node => node.span.kind !== "work.session").map(labelBox);
    this.arrow = `${chart.id}-return-arrow`;
    this.clip = this.projection.cuts.length ? `url(#${chart.id}-visible-time)` : null;
  }
  draw(): void {
    const { height } = this.projection;
    attrs(this.svg, { viewBox: `0 0 ${this.fullWidth} ${height}`, width: this.fullWidth, height });
    drawFlowGuides(this.svg, this.projection, this.fullWidth, this.chart.id);
    const edges = this.svg.append("g").attr("class", "flow-edges");
    for (const node of this.nodes) {
      const at = dispatchTime(node.span);
      if (at !== null && connected(node, at) && !node.span.conflicts?.length) this.connector(edges, node, at, "dispatch");
      const back = returnTime(node.span);
      if (back !== null && connected(node, back)) this.connector(edges, node, back, "return");
    }
    for (const node of this.nodes) this.drawNode(node);
  }
  private connector(group: Group, node: FlowNode, at: number, kind: "dispatch" | "return"): void {
    const parent = node.parent!.x + 6, child = node.x - 6, y = this.projection.y(at);
    const [from, to] = kind === "return" ? [child, parent] : [parent, child];
    element(group, "path", { class: `flow-${kind}`, "data-at": at, "data-owner": node.span.id, d: flowLink(from, to, y, this.labelBoxes), "stroke-linejoin": "round", stroke: "var(--muted)", "stroke-width": 1.2, fill: "none", "marker-end": `url(#${this.arrow})` });
  }
  private drawNode(node: FlowNode): void {
    const { span, x } = node;
    const group = this.svg.append("g").attr("class", `waterfall-node${span.id === this.chart.selected ? " selected" : ""}`).attr("data-node-id", span.id).attr("data-x", x);

    this.intervals(group.append("g").attr("clip-path", this.clip), node);
    this.label(group, node);
    const purpose = (span.operations ?? []).map(operation => `${operation.category_name} (${operation.evidence})`).join("; ");
    const detail = `${span.label} · ${span.kind} · ${duration(span)}${qualification(node)} · ${span.role ?? "unknown"}${purpose ? ` · ${purpose}` : ""}`;
    element(group, "title", {}, detail);
    group.attr("aria-label", detail);
    actionable(group, span.id, this.choose);
  }
  private intervals(group: Group, node: FlowNode): void {
    const { span, x } = node, { y, horizon } = this.projection;
    if (span.queued !== null) this.stripe(group, node, { start: span.queued, end: span.start ?? span.end ?? horizon, state: "wait" }, "flow-queue");
    if (span.end === null && span.role !== "container") {
      const at = span.start ?? span.queued;
      if (at !== null) element(group, "circle", { class: "flow-open-boundary", "data-at": at, cx: x, cy: y(at), r: 5, fill: "none", stroke: "var(--muted)", "stroke-dasharray": "2 2" });
    } else {
      for (const segment of segments(node, this.nodes, horizon)) this.stripe(group, node, segment);
    }
    if (span.end !== null && !span.conflicts?.length) element(group, "line", { class: "flow-end", "data-at": span.end, x1: x - 9, x2: x + 9, y1: y(span.end), y2: y(span.end), stroke: color(span.kind), "stroke-width": 3 });
    if (span.start === null && span.queued === null && span.end !== null) element(group, "circle", { cx: x, cy: y(span.end), r: 5, fill: "none", stroke: color(span.kind) });
  }
  private label(group: Group, node: FlowNode): void {
    const { span, x, labelY } = node;
    if (span.kind === "work.session") return;
    const peers = this.nodes.filter(other => other.span.label === span.label);
    const sourceLabel = peers.length > 1 ? `${span.label} · ${peers.indexOf(node) + 1}` : span.label;
    const label = truncateLabel(sourceLabel);
    const anchor = span.start ?? span.queued ?? span.end;
    if (anchor !== null && Math.abs(labelY - this.projection.y(anchor)) > 18) {
      element(group, "path", { class: "flow-label-leader", d: `M${x + 7},${this.projection.y(anchor)}L${x + 16},${labelY - 4}`, stroke: "var(--muted)", fill: "none", "stroke-width": .8 });
    }
    element(group, "rect", { class: "flow-label-ground", x: x + 13, y: labelY - 14, width: node.labelWidth, height: 38, rx: 2, fill: "var(--ground)" });
    element(group, "text", { class: "chart-label flow-label", x: x + 20, y: labelY }, label);
    const bucket = activityBucket(span);
    const meta = `${duration(span)}${qualification(node)}${bucket ? ` · ${bucket}` : ""}`;
    element(group, "text", { class: "chart-label flow-meta", x: x + 20, y: labelY + 17 }, truncateLabel(meta));
    if (span.start === null && span.queued === null && span.end === null) element(group, "text", { class: "chart-label", x, y: labelY + 46 }, "Source boundaries unavailable");
  }
  private stripe(group: Group, node: FlowNode, segment: FlowSegment, extra = ""): void {
    const { start, end, state } = segment, ink = color(node.span.kind), y = this.projection.y;
    if (end < start) return;
    element(group, "rect", { class: `flow-${state} ${extra}`, "data-start": start, "data-end": end, x: node.x - 6, y: y(start), width: 12, height: Math.max(2, y(end) - y(start)), rx: 1, fill: state === "active" ? ink : "none", stroke: state === "unknown" ? "var(--muted)" : ink, "stroke-width": 1.3, "stroke-dasharray": state === "wait" ? "2 4" : state === "unknown" ? "6 4" : null, "stroke-opacity": state === "unknown" ? .55 : 1 });
  }
}
