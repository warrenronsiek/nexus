// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import type { Placed, Span } from "./chart-model";

export interface WaterfallLayout { origin: number; end: number; nodes: Placed[]; lanes: number }
/** Canonical parentage determines ancestry. Source intervals determine lane reuse. */
export function layoutWaterfall(spans: Span[]): WaterfallLayout {
  return new LaneLayout(spans).build();
}
class LaneLayout {
  private readonly indexed: Map<string, Span>;
  private readonly placed = new Map<string, Placed>();
  private readonly lanes: Placed[][] = [];
  private readonly origin: number;
  private readonly end: number;
  constructor(private readonly spans: Span[]) {
    this.indexed = new Map(spans.map(span => [span.id, span]));
    const times = spans.flatMap(span => [span.queued, span.start, span.end]).filter((time): time is number => time !== null && Number.isFinite(time));
    this.origin = times.length ? Math.min(...times) : 0;
    this.end = times.length ? Math.max(...times) : 0;
  }
  build(): WaterfallLayout {
    this.spans.forEach(span => this.place(span, new Set()));
    return { origin: this.origin, end: this.end, nodes: [...this.placed.values()], lanes: this.lanes.length };
  }
  private parent(span: Span, seen: Set<string>): Placed | undefined {
    if (span.parent === null || seen.has(span.parent)) return undefined;
    const source = this.indexed.get(span.parent);
    return source ? this.place(source, new Set([...seen, span.id])) : undefined;
  }
  private overlaps(span: Span, other: Placed): boolean {
    const start = span.queued ?? span.start ?? this.origin, end = span.end ?? this.end;
    return start <= (other.end ?? this.end) && end >= (other.queued ?? other.start ?? this.origin);
  }
  private freeLane(minimum: number, span: Span): number {
    let lane = minimum;
    while (this.lanes[lane]?.some(other => this.overlaps(span, other))) lane++;
    return lane;
  }
  private place(span: Span, seen: Set<string>): Placed {
    const existing = this.placed.get(span.id);
    if (existing) return existing;
    const parent = this.parent(span, seen);
    const minimum = parent ? parent.lane + 1 : Number(span.parent !== null);
    const lane = this.freeLane(minimum, span);
    const result = { ...span, lane, parentLane: parent?.lane ?? null, unresolved: span.parent !== null && !parent };
    (this.lanes[lane] ??= []).push(result);
    this.placed.set(span.id, result);
    return result;
  }
}
