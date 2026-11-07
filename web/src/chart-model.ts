// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
// @boundary chart-ports: Elm owns domain state; this protocol contains display inputs and selection identities only.
export interface SpanEvidence { executionQualified?: boolean; executor?: string | null; role?: "container" | "execution" | "wait" | "unknown"; callMode?: "synchronous" | "asynchronous" | "unknown"; returnedAt?: number | null; conflicts?: string[]; gaps?: string[] }
export interface Span extends SpanEvidence { id: string; parent: string | null; label: string; kind: string; start: number | null; end: number | null; queued: number | null }
export interface Point { id: string; label: string; at: number; value: number | null }
export interface Bar { id: string; label: string; value: number | null; detail: string }
export interface Bin { start: number; count: number }
export interface Chart { id: string; type: "bars" | "trend" | "waterfall" | "histogram"; bars?: Bar[]; points?: Point[]; spans?: Span[]; bins?: Bin[]; binSeconds?: number; line?: boolean; selected?: string | null; zoom?: number }
export interface ChartInput extends Omit<Chart, "points" | "spans" | "bins"> { points?: (Omit<Point, "at"> & { at: string | number })[]; spans?: (Omit<Span, "start" | "end" | "queued" | "returnedAt"> & { returnedAt?: string | number | null; start: string | number | null; end: string | number | null; queued: string | number | null })[]; bins?: { start: string | number; count: number }[] }
export interface ChartEvent { chart: string; id: string }
export type Choose = (id: string) => void;

function timestamp(value: string | number | null): number | null {
  if (value === null) return null;
  const parsed = typeof value === "string" ? Date.parse(value) : value;
  return Number.isFinite(parsed) ? parsed : null;
}
function positionedSpan(span: NonNullable<ChartInput["spans"]>[number]): Span {
  return { ...span, returnedAt: timestamp(span.returnedAt ?? null), start: timestamp(span.start), end: timestamp(span.end), queued: timestamp(span.queued) };
}
export function positionedChart(input: ChartInput): Chart {
  const points = input.points?.map(point => ({ ...point, at: timestamp(point.at) ?? NaN }));
  const bins = input.bins?.map(bin => ({ count: bin.count, start: timestamp(bin.start) ?? NaN }));
  return { ...input, points, bins, spans: input.spans?.map(positionedSpan) };
}
