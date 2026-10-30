// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { layoutWaterfall, renderWaterfall, type WaterfallNode } from "./work-waterfall.ts";
import { waterfallMetadata } from "./work-waterfall-metadata.ts";
import { activityStyle } from "./work-activity-style.ts";
import { chartBars, chartTrend, formatDurationSeconds } from "./charts.ts";
import { centeredWindowStart, fitDisplayText } from "./display.ts";
import type { WorkDashboard, WorkKindStats } from "./work-dashboard-wire.ts";
import type { WorkDashboardView } from "./work-state.ts";
import type { WorkSummary } from "./work.ts";
import type { DashboardTheme } from "./theme.ts";

export function dashboardItems(view: WorkDashboardView): Pick<WorkSummary, "subject" | "title">[] {
  if (!view.sessionId) return view.data?.sessions ?? [];
  const root = view.data?.selected_session;
  return [...(root ? [root] : []), ...waterfallNodes(view.data).map(node => ({ subject: { scope: "activity" as const, id: node.id }, title: node.title }))];
}
export function waterfallNodes(data?: WorkDashboard): WaterfallNode[] {
  return data?.intervals ?? data?.activities.map(item => ({ ...item, id: item.subject.id, executor: null })) ?? [];
}
export function sessionWaterfall(data: WorkDashboard) {
  const session = data.selected_session!;
  return layoutWaterfall({ ...session, id: session.subject.id, executor: null }, waterfallNodes(data));
}
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
    ...chartTrend(points, width, formatDurationSeconds).slice(0, 6)];
}
function notices(data: WorkDashboard): string[] {
  return [...(data.truncated ? ["Partial history · charts summarize retained subjects only"] : []),
    ...data.notices.slice(0, 2), ...(data.notices.length > 2 ? [`${data.notices.length - 2} additional notices · d browse source records`] : [])];
}
function overview(view: WorkDashboardView, data: WorkDashboard, width: number): string[] {
  const stats = data.session_duration;
  const start = centeredWindowStart(data.sessions.length, view.selected, 3);
  return ["Work duration dashboard · d source records", `Session wall · avg ${duration(stats.average_seconds)} · total ${duration(stats.known_count ? stats.total_seconds : null)} · ${stats.known_count}/${stats.count} sessions timed`,
    "inclusive intervals; not effort. Unknown/conflicted excluded.",
    ...trend(view, data, width), ...kindChart(view, data, width),
    `Harness sessions · Enter visual breakdown · ↑/↓ select · ${data.sessions.length} retained`,
    ...data.sessions.slice(start, start + 3).map((item, offset) => fitDisplayText(`${start + offset === view.selected ? "›" : " "} ${item.title} · ${elapsed(item)} · ${item.outcome ?? item.state}`, width)),
    ...(data.sessions.length ? [] : ["No harness sessions recorded. Check host capture; d browses all source records."]), ...notices(data).slice(0, 3)];
}
function sessionBreakdown(view: WorkDashboardView, data: WorkDashboard, width: number, theme?: DashboardTheme): string[] {
  const session = data.selected_session;
  if (!session) return ["Selected session is unavailable · Escape returns to overview."];
  const nodes = waterfallNodes(data), subject = dashboardItems(view)[view.selected]?.subject;
  const selected = subject?.scope === "session" ? { ...session, id: session.subject.id, executor: null } : nodes.find(node => node.id === subject?.id);
  const layout = sessionWaterfall(data), origin = Number.isFinite(layout.origin) ? new Date(layout.origin).toISOString() : "unknown";
  const metadata = selected && subject ? waterfallMetadata(selected, subject.scope).map(line => fitDisplayText(line, width)) : ["Selected task unavailable · refresh source history."];
  if (selected && theme) metadata[0] = theme.bold(theme.fg(activityStyle(selected.kind).color, metadata[0]!));
  return [...[`Session duration breakdown · ${session.title}`, `Session elapsed ${elapsed(session)} · UTC origin ${origin} · ${data.truncated ? "Retained" : "All recorded"} descendants · ${nodes.length} tasks`,
    ...accountingLines(data)].map(line => fitDisplayText(line, width)),
    ...renderWaterfall(layout, view.waterfall, width, { selected: subject, theme, rows: Math.max(6, Math.min(24, (process.stdout.rows ?? 32) - 26)) }),
    ...metadata,
    ...(data.truncated ? [`Retained descendants · ${nodes.length} tasks · PARTIAL`] : []),
    ...notices(data).slice(0, 1).map(line => fitDisplayText(line, width))];
}
function accountingLines(data: WorkDashboard): string[] {
  const accounting = data.accounting;
  if (!accounting) return ["Accounting unavailable · missing source inventory prevents complete coverage claims"];
  const coverage = accounting.coverage;
  return [`Attributed ${duration(accounting.occupied_wall?.seconds ?? null)} · unobserved ${duration(accounting.unobserved?.seconds ?? null)} · queue sum ${queueDuration(accounting)} · worker occupancy ${duration(accounting.worker_occupancy?.seconds ?? null)} · not effort`,
    `Coverage · start missing ${coverage.missing_start_count} · finish missing ${coverage.missing_finish_count} · clocks ${coverage.clock_conflict_count} · role? ${coverage.unknown_role_count} · inventory ${coverage.capability_inventory}${coverage.source_partial || coverage.display_partial ? " · PARTIAL" : ""}`];
}
function queueDuration(accounting: NonNullable<WorkDashboard["accounting"]>): string {
  const { queue_count, known_queue_count } = accounting.coverage;
  const value = queue_count && known_queue_count === 0 ? null : accounting.queue_wait?.seconds ?? null;
  return `${duration(value)}${queue_count === undefined ? "" : ` · ${known_queue_count}/${queue_count} timed`}`;
}
/** Source strings are fitted/sanitized before trusted theme ANSI is applied. */
export function renderWorkDashboard(view: WorkDashboardView, width: number, status: string, theme?: DashboardTheme): string[] {
  if (!view.data) return [view.sessionId ? "Session duration breakdown" : "Work duration dashboard", status].map(line => fitDisplayText(line, width));
  const lines = view.sessionId ? sessionBreakdown(view, view.data, width, theme) : overview(view, view.data, width).map(line => fitDisplayText(line, width));
  const health = view.captureHealth;
  const capture = health?.available ? `Local capture · queued ${health.queued} · retained rejected ${health.rejected} · lost ${health.lost ?? "unknown"} · process losses ${health.process_losses} · health failures ${health.health_failures}` : "Local capture health unavailable · delivery completeness unknown";
  const retention = health?.available ? [`Callback rejected ${health.rejected_callbacks ?? "unknown"} · Transport rejected ${health.transport_rejected ?? "unknown"} · Capacity losses ${health.legacy_capacity_losses ?? "unknown"} · rejected history evicted ${health.rejected_evicted ?? "unknown"} · pending callbacks ${health.pending_callbacks ?? "unknown"} · local counters`] : [];
  return [...lines, ...[capture, ...retention].map(line => fitDisplayText(line, width))];
}
