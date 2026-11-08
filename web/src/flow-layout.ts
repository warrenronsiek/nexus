// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import type { Span } from "./chart-model";
export function dispatchTime(span: Span): number | null {
  const boundary = span.start ?? span.end;
  return span.queued !== null && (boundary === null || span.queued <= boundary) ? span.queued : span.start;
}
export interface FlowNode { span: Span; x: number; labelY: number; parent: FlowNode | null; labelWidth: number }
export interface Box { left: number; right: number; top: number; bottom: number }
const pitch = 72;
function intersects(a: Box, b: Box): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}
export function labelBox(node: FlowNode): Box {
  return { left: node.x + 13, right: node.x + 13 + node.labelWidth, top: node.labelY - 14, bottom: node.labelY + 24 };
}
function boxes(node: FlowNode, y: (at: number) => number, horizon: number): Box[] {
  const start = dispatchTime(node.span) ?? node.span.end ?? horizon;
  const label = labelBox(node);
  return [
    { left: node.x - 12, right: node.x + 12, top: y(start) - 5, bottom: y(node.span.end ?? (node.span.role === "container" ? horizon : start)) + 8 },
    { left: label.left - 1, right: label.right + 7, top: label.top - 8, bottom: label.bottom + 4 },
  ];
}
function placeNode(span: Span, parent: FlowNode | null, occupied: ObstacleIndex, y: (at: number) => number, horizon: number): FlowNode {
  const top = dispatchTime(span) ?? span.end;
  const labelY = top === null ? 88 : Math.max(86, y(top) - 12);
  const metadataWidth = ((span.operations ?? []).map(operation => operation.category_name).join(", ").length + 8) * 6.2;
  const node = { span, parent, x: parent ? parent.x + pitch : 140, labelY, labelWidth: Math.max(120, Math.min(184, Math.max(span.label.length * 7.4, metadataWidth))) };
  if (span.kind === "work.session" && parent === null) { node.x = 90; return node; }
  if (parent?.span.kind === "work.session") node.x = parent.x + 50;
  for (;;) {
    for (const offset of [0, 42, -42, 84]) {
      node.labelY = Math.max(64, labelY + offset);
      if (!occupied.collides(boxes(node, y, horizon))) return node;
    }
    node.x += 60;
  }
}
/** Collision avoidance only changes horizontal position; source-time coordinates never move. */
export function layoutFlow(spans: Span[], y: (at: number) => number, horizon: number): FlowNode[] {
  const byId = new Map(spans.map(span => [span.id, span])), placed = new Map<string, FlowNode>();
  const nodes: FlowNode[] = [], occupied = new ObstacleIndex();
  const sorted = [...spans].sort((a, b) => sourceTime(a) - sourceTime(b) || a.id.localeCompare(b.id));
  for (const span of sorted) {
    const chain = ancestorChain(span, byId, placed);
    for (const source of chain.reverse()) {
      const parent = source.parent ? placed.get(source.parent) ?? null : null;
      const node = placeNode(source, parent, occupied, y, horizon);
      nodes.push(node); placed.set(source.id, node);
      if (source.kind !== "work.session") occupied.add(boxes(node, y, horizon));
    }
  }
  return nodes;
}
function sourceTime(span: Span): number { return dispatchTime(span) ?? span.end ?? Infinity; }

function ancestorChain(span: Span, byId: Map<string, Span>, placed: Map<string, FlowNode>): Span[] {
  const chain: Span[] = [], visiting = new Set<string>();
  let current: Span | undefined = span;
  while (current && !placed.has(current.id) && !visiting.has(current.id)) {
    visiting.add(current.id); chain.push(current);
    current = current.parent ? byId.get(current.parent) : undefined;
  }
  return chain;
}

/** Broad-phase spatial index keeps dense source bursts from scanning every earlier label. */
class ObstacleIndex {
  private columns = new Map<number, Box[]>();
  add(boxes: Box[]): void {
    for (const box of boxes) {
      for (let col = Math.floor(box.left / 60); col <= Math.floor(box.right / 60); col++) {
        const existing = this.columns.get(col) ?? [];
        existing.push(box); this.columns.set(col, existing);
      }
    }
  }
  collides(boxes: Box[]): boolean {
    for (const box of boxes) {
      for (let col = Math.floor(box.left / 60); col <= Math.floor(box.right / 60); col++) {
        if (this.columns.get(col)?.some(other => intersects(box, other))) return true;
      }
    }
    return false;
  }
}
