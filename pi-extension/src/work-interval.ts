// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import type { WorkSummary } from "./work.ts";

export interface WorkTimeAxis { start: number; end: number; columns: number }
/** Callers choose the visible population and width; source boundaries share one layout mechanism. */
export function workTimeAxis(boundaries: readonly (string | null)[], columns: number): WorkTimeAxis | undefined {
  const times = boundaries.filter((value): value is string => value !== null).map(Date.parse).filter(Number.isFinite);
  return times.length ? { start: Math.min(...times), end: Math.max(...times), columns } : undefined;
}
export function workTimeBar(item: WorkSummary, axis: WorkTimeAxis | undefined): string {
  if (!axis || item.started_at === null) return "[unknown]";
  const start = Date.parse(item.started_at), end = item.finished_at === null ? null : Date.parse(item.finished_at);
  if (!Number.isFinite(start) || end !== null && (!Number.isFinite(end) || end < start)) return "[clock anomaly]";
  const position = (time: number): number => Math.round((time - axis.start) / Math.max(1, axis.end - axis.start) * (axis.columns - 1));
  const from = position(start), to = end === null ? from : position(end);
  return `[${Array.from({ length: axis.columns }, (_, index) => index < from || index > to ? " " : end === null ? "?" : "=").join("")}]`;
}
