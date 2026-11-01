// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
// @boundary chart-ports: Elm owns domain state; this protocol contains display inputs and selection identities only.
export interface Span { id: string; parent: string | null; label: string; kind: string; start: number | null; end: number | null; queued: number | null }
export interface Placed extends Span { lane: number; parentLane: number | null; unresolved: boolean }
export interface Point { id: string; label: string; at: number; value: number | null }
export interface Bar { id: string; label: string; value: number | null; detail: string }
export interface Chart { id: string; type: "bars" | "trend" | "waterfall"; bars?: Bar[]; points?: Point[]; spans?: Span[]; selected?: string | null; zoom?: number }
export interface ChartInput extends Omit<Chart, "points" | "spans"> { points?: (Omit<Point, "at"> & { at: string | number })[]; spans?: (Omit<Span, "start" | "end" | "queued"> & { start: string | number | null; end: string | number | null; queued: string | number | null })[] }
export interface ChartEvent { chart: string; id: string }
export type Choose = (id: string) => void;

function timestamp(value: string | number | null): number | null {
  if (value === null) return null;
  const parsed = typeof value === "string" ? Date.parse(value) : value;
  return Number.isFinite(parsed) ? parsed : null;
}
function positionedSpan(span: NonNullable<ChartInput["spans"]>[number]): Span {
  return { ...span, start: timestamp(span.start), end: timestamp(span.end), queued: timestamp(span.queued) };
}
export function positionedChart(input: ChartInput): Chart {
  const points = input.points?.map(point => ({ ...point, at: timestamp(point.at) ?? NaN }));
  return { ...input, points, spans: input.spans?.map(positionedSpan) };
}
