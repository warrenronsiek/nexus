// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { workTimeAxis, workTimeBar } from "./work-interval.ts";
import { chartBars, chartTrend, formatDurationSeconds } from "./charts.ts";
import { centeredWindowStart, fitDisplayText, sanitizeDisplayText } from "./display.ts";
import type { WorkDashboard, WorkKindStats } from "./work-dashboard-wire.ts";
import type { WorkDashboardView } from "./work-state.ts";
import type { WorkSummary } from "./work.ts";

export function dashboardItems(view: WorkDashboardView): WorkSummary[] { return view.sessionId ? descendantRows(view.data?.activities ?? []).map(row => row.summary) : view.data?.sessions ?? []; }
function duration(value: number | null): string { return value === null ? "unknown" : formatDurationSeconds(value); }
function elapsed(item: WorkSummary): string {
  const value = item.elapsed ? Number(item.elapsed.nanoseconds) / 1e9 : NaN;
  return Number.isFinite(value) && !item.conflicts.length ? `${formatDurationSeconds(value)} (${item.elapsed!.method})` : "unknown";
}
function metric(kind: WorkKindStats, view: WorkDashboardView): number | null { return kind.known_count === 0 ? null : view.metric === "average" ? kind.average_seconds : kind.total_seconds; }
function kindChart(view: WorkDashboardView, data: WorkDashboard, width: number): string[] {
  const start = centeredWindowStart(data.activity_kinds.length, view.kindIndex, 3);
  const rows = data.activity_kinds.slice(start, start + 3).map((kind, index) => ({
    label: `${start + index === view.kindIndex ? "› " : ""}${kind.kind}`, value: metric(kind, view), valueLabel: `${duration(metric(kind, view))} · ${kind.known_count}/${kind.count} timed`,
  }));
  return [`${view.metric === "average" ? "Average" : "Total"} activity duration by kind · m metric · [/]`,
    ...(rows.length ? chartBars(rows, width) : ["No recorded activity intervals."]),
    ...(data.activity_kinds.length > 3 ? [`Kinds ${start + 1}–${Math.min(start + 3, data.activity_kinds.length)} of ${data.activity_kinds.length}`] : [])];
}
function trend(view: WorkDashboardView, data: WorkDashboard, width: number): string[] {
  const kind = data.activity_kinds[view.kindIndex]?.kind;
  const activities = view.trend === "activities";
  const samples = activities ? data.activity_daily.filter(day => day.kind === kind) : data.daily;
  const points = samples.map(day => ({ label: day.date, at: Date.parse(day.date), value: day.known_count === 0 ? null : view.metric === "average" ? day.average_seconds : day.total_seconds }));
  return [`${activities ? `${kind ?? "Activity"} durations` : "Session durations"} over time · UTC source day · ${view.metric} · t series`,
    ...chartTrend(points, width, formatDurationSeconds).slice(0, 7)];
}
function notices(data: WorkDashboard): string[] {
  return [...(data.truncated ? ["Partial history · charts summarize retained subjects only"] : []),
    ...data.notices.slice(0, 2), ...(data.notices.length > 2 ? [`${data.notices.length - 2} additional notices · d browse source records`] : [])];
}
function overview(view: WorkDashboardView, data: WorkDashboard, width: number): string[] {
  const stats = data.session_duration;
  const start = centeredWindowStart(data.sessions.length, view.selected, 4);
  return ["Work duration dashboard · d source records", `Session wall · avg ${duration(stats.average_seconds)} · total ${duration(stats.known_count ? stats.total_seconds : null)} · ${stats.known_count}/${stats.count} sessions timed`,
    "inclusive intervals; not effort. Unknown/conflicted excluded.",
    ...trend(view, data, width), ...kindChart(view, data, width),
    `Parent sessions · Enter visual breakdown · ↑/↓ select · ${data.sessions.length} retained`,
    ...data.sessions.slice(start, start + 4).map((item, offset) => fitDisplayText(`${start + offset === view.selected ? "›" : " "} ${item.title} · ${elapsed(item)} · ${item.outcome ?? item.state}`, width)),
    ...(data.sessions.length ? [] : ["No work sessions recorded. Capture caller-owned session and activity boundaries."]), ...notices(data)];
}
interface DescendantRow { summary: WorkSummary; depth: number }
/** Walk canonical ownership once; missing parents and graph conflicts remain visible at the root. */
function descendantRows(items: WorkSummary[]): DescendantRow[] {
  const children = childrenByParent(items), rows: DescendantRow[] = [], visited = new Set<string>();
  const append = (summary: WorkSummary, depth: number): void => {
    if (visited.has(summary.subject.id) || depth > 128) return;
    visited.add(summary.subject.id); rows.push({ summary, depth });
    for (const child of children.get(summary.subject.id) ?? []) append(child, depth + 1);
  };
  for (const item of children.get(null) ?? []) append(item, 0);
  for (const item of items) if (!visited.has(item.subject.id)) append(item, 0);
  return rows;
}
function childrenByParent(items: WorkSummary[]): Map<string | null, WorkSummary[]> {
  const ids = new Set(items.map(item => item.subject.id)), children = new Map<string | null, WorkSummary[]>();
  for (const item of items) {
    const parent = item.parent_activity_id !== null && ids.has(item.parent_activity_id) ? item.parent_activity_id : null;
    const siblings = children.get(parent) ?? []; siblings.push(item); children.set(parent, siblings);
  }
  return children;
}

function timeline(view: WorkDashboardView, data: WorkDashboard, session: WorkSummary, width: number): string[] {
  const rows = descendantRows(data.activities), start = centeredWindowStart(rows.length, view.selected, 8);
  const boundaries = [session, ...data.activities].flatMap(item => [item.started_at, item.finished_at]);
  const axis = workTimeAxis(boundaries, Math.min(28, Math.max(5, Math.floor(width / 4))));
  return [`${data.truncated ? "Retained descendants" : "All recorded descendants"} · ${rows.length} activities · ↑/↓ scroll · Enter source detail`,
    axis ? `UTC axis ${new Date(axis.start).toISOString()} → ${new Date(axis.end).toISOString()} · = source interval · ? unknown` : "UTC axis unavailable · source boundaries missing",
    ...rows.slice(start, start + 8).map((row, offset) => fitDisplayText(`${start + offset === view.selected ? "›" : " "} ${row.summary.conflicts.length ? "[? unknown]" : workTimeBar(row.summary, axis)} ${"  ".repeat(Math.min(8, row.depth))}${sanitizeDisplayText(row.summary.title)} · ${elapsed(row.summary)}${row.summary.gaps.length || row.summary.conflicts.length ? " · gaps/conflicts" : ""}`, width)),
    ...(rows.length > 8 ? [`Activities ${start + 1}–${Math.min(start + 8, rows.length)} of ${rows.length}`] : []),
    ...(rows.length ? [] : ["No descendant intervals captured; absent child internals remain unknown."])];
}
function sessionBreakdown(view: WorkDashboardView, data: WorkDashboard, width: number): string[] {
  const session = data.selected_session;
  if (!session) return ["Selected session is unavailable · Escape returns to overview."];
  return [`Session duration breakdown · ${session.title}`, `Session elapsed ${elapsed(session)} · ${session.outcome ?? session.state} · s detail · Esc back`,
    `Session gaps: ${session.gaps.join(", ") || "none reported"} · conflicts: ${session.conflicts.join(", ") || "none reported"}`,
    "Nested/parallel intervals overlap; sums are not elapsed or effort.",
    ...kindChart(view, data, width), ...timeline(view, data, session, width), ...notices(data)];
}
export function renderWorkDashboard(view: WorkDashboardView, width: number, status: string): string[] {
  if (!view.data) return [view.sessionId ? "Session duration breakdown" : "Work duration dashboard", status];
  const lines = view.sessionId ? sessionBreakdown(view, view.data, width) : overview(view, view.data, width);
  return lines.map(line => fitDisplayText(line, width));
}
