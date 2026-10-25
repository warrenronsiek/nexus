// @feature observability-ui
// @spec docs/features/observability-ui.md
import { visibleWidth } from "@earendil-works/pi-tui";
import { fitDisplayText as fit, sanitizeDisplayText } from "./display.ts";

export interface ChartBar { label: string; value: number | null; valueLabel: string }
export interface ChartPoint { label: string; value: number | null; at?: number }

function pad(text: string, width: number): string { return text + " ".repeat(Math.max(0, width - visibleWidth(text))); }
function numeric(value: number | null): value is number { return value !== null && Number.isFinite(value); }
interface BarLayout { available: number; maximum: number; label: number; value: number; bar: number }
function barLayout(rows: readonly ChartBar[], width: number): BarLayout {
  const available = Math.max(1, Math.floor(width));
  const maximum = Math.max(1e-300, ...rows.map(row => numeric(row.value) ? Math.abs(row.value) : 0));
  const fieldWidth = (values: string[]): number => Math.min(Math.max(1, Math.floor(available / 3)), Math.max(1, ...values.map(value => visibleWidth(sanitizeDisplayText(value)))));
  const label = fieldWidth(rows.map(row => row.label)), value = fieldWidth(rows.map(row => row.valueLabel));
  return { available, maximum, label, value, bar: Math.max(1, available - label - value - 2) };
}

/** Display-only scaling; the caller retains exact values and owns their meaning. */
export function chartBars(rows: readonly ChartBar[], width: number): string[] {
  const layout = barLayout(rows, width);
  return rows.map(row => {
    const label = pad(fit(row.label, layout.label), layout.label);
    const value = row.value;
    const filled = numeric(value) && value !== 0 ? Math.max(1, Math.round(Math.abs(value) / layout.maximum * layout.bar)) : 0;
    const bar = numeric(value) ? `${value < 0 ? "−" : ""}${"█".repeat(filled)}` : "?";
    return fit(`${label} ${pad(fit(bar, layout.bar), layout.bar)} ${row.valueLabel}`, layout.available);
  });
}

interface TrendPlot { minimum: number; maximum: number; rows: string[]; timed: boolean }
function trendPlot(points: readonly ChartPoint[], columns: number): TrendPlot {
  const values = points.map(point => point.value).filter(numeric);
  const minimum = Math.min(...values), maximum = Math.max(...values);
  const layout = trendPositions(points, columns);
  const rows = Array.from({ length: 4 }, () => Array<string>(layout.timed ? columns : points.length).fill(" "));
  points.forEach((point, index) => {
    const column = layout.positions[index]!;
    if (!numeric(point.value)) { rows[3]![column] = "?"; return; }
    const row = maximum === minimum ? 1 : Math.round((maximum - point.value) / (maximum - minimum) * 3);
    rows[row]![column] = "●";
  });
  return { minimum, maximum, rows: rows.map(row => row.join("")), timed: layout.timed };
}

function trendPositions(points: readonly ChartPoint[], columns: number): { positions: number[]; timed: boolean } {
  const times = points.map(point => point.at);
  if (!times.every((at): at is number => at !== undefined && Number.isFinite(at))) return { positions: points.map((_, index) => index), timed: false };
  const first = Math.min(...times), last = Math.max(...times);
  return { positions: times.map(time => last === first ? 0 : Math.round((time - first) / (last - first) * (columns - 1))), timed: true };
}

/** Dated points share a proportional UTC axis. Gaps remain gaps; values are never interpolated. */
export function chartTrend(points: readonly ChartPoint[], width: number, formatValue: (value: number) => string): string[] {
  const available = Math.max(1, Math.floor(width));
  const columns = Math.max(1, available - 12);
  const shown = points.slice(-columns);
  const values = shown.map(point => point.value).filter(numeric);
  if (!values.length) return [fit("No numeric observations · ? unknown", available)];
  const axisWidth = Math.min(Math.max(1, available - 2), 10);
  const plot = trendPlot(shown, columns);
  const lines = plot.rows.map((marks, row) => {
    const axis = row === 0 ? formatValue(plot.maximum) : row === 3 ? formatValue(plot.minimum) : "";
    return fit(`${pad(fit(axis, axisWidth), axisWidth)}│${marks}`, available);
  });
  const first = shown[0]!.label, last = shown.at(-1)!.label;
  lines.push(fit(`${plot.timed ? "Time axis · " : "Sequence · "}${first === last ? first : `${first} → ${last}`}`, available));
  if (shown.length < points.length) lines.push(fit(`Latest ${shown.length} of ${points.length} points`, available));
  if (shown.some(point => !numeric(point.value))) lines.push(fit("? missing / nonnumeric observation", available));
  return lines;
}

export function formatDurationSeconds(seconds: number): string {
  const [scale, unit] = seconds < 1 ? [0.001, "ms"] : seconds < 60 ? [1, "s"] : seconds < 3600 ? [60, "m"] : [3600, "h"];
  return `${Number((seconds / scale).toPrecision(3))}${unit}`;
}
