// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import type { Span } from "./chart-model";
import { dispatchTime, type FlowNode } from "./flow-layout";
export interface FlowSegment { start: number; end: number; state: "active" | "wait" | "unknown" }
export function qualified(span: Span): boolean {
  return span.executionQualified === true && span.start !== null && span.end !== null && span.end >= span.start;
}
export function returnTime(span: Span): number | null {
  if (!qualified(span)) return null;
  return span.returnedAt != null && span.returnedAt >= span.end! ? span.returnedAt : null;
}
export function connected(node: FlowNode, at: number): boolean {
  const parent = node.parent?.span;
  return !!parent && !parent.conflicts?.length && parent.start !== null && at >= parent.start &&
    (parent.end === null || at <= parent.end);
}
function observedState(node: FlowNode): FlowSegment["state"] {
  if (!qualified(node.span)) return "unknown";
  if (node.span.role === "execution") return "active";
  return node.span.role === "wait" ? "wait" : "unknown";
}
function childSegments(node: FlowNode, children: FlowNode[]): FlowSegment[] {
  return children.filter(child => child.parent === node && qualified(child.span) &&
    connected(child, dispatchTime(child.span)!) && connected(child, child.span.end!))
    .flatMap(child => {
      const sameWorker = !!node.span.executor && node.span.executor === child.span.executor;
      const state = child.span.callMode === "synchronous" ? "wait" : sameWorker ? observedState(child) : "unknown";
      const start = child.span.callMode === "synchronous" ? dispatchTime(child.span)! : child.span.start!;
      return state === "unknown" ? [] : [{ start, end: child.span.end!, state }];
    });
}
/** Only qualified observations paint a container; unknown gaps never become model activity. */
export function segments(node: FlowNode, children: FlowNode[], horizon: number): FlowSegment[] {
  const start = node.span.start;
  if (start === null) return [];
  const end = node.span.end ?? Math.max(start, horizon);
  if (end < start) return [];
  const observed = childSegments(node, children);
  const boundaries = [...new Set([start, end, ...observed.flatMap(item => [item.start, item.end])])].sort((a, b) => a - b);
  const result: FlowSegment[] = [], base = observedState(node);
  for (let i = 1; i < boundaries.length; i++) {
    const a = boundaries[i - 1], b = boundaries[i];
    const state = stateDuring(observed, a, b, base);
    appendSegment(result, { start: a, end: b, state });
  }
  return result.length ? result : [{ start, end, state: base }];
}

function appendSegment(result: FlowSegment[], segment: FlowSegment): void {
  const previous = result.at(-1);
  if (previous?.state === segment.state) previous.end = segment.end;
  else result.push(segment);
}

function stateDuring(observed: FlowSegment[], start: number, end: number, fallback: FlowSegment["state"]): FlowSegment["state"] {
  const covering = observed.filter(item => start >= item.start && end <= item.end);
  return covering.some(item => item.state === "wait") ? "wait" : covering[0]?.state ?? fallback;
}
