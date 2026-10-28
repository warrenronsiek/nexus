// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
// @boundary work-dashboard-wire: validate scoped aggregate projections before display; exact source evidence remains in work detail.
import type { WorkSummary } from "./work.ts";
import type { WaterfallNode } from "./work-waterfall.ts";
import { boundedArray as array } from "./wire-values.ts";

export interface WorkDurationStats { count: number; known_count: number; total_seconds: number; average_seconds: number | null }
export interface WorkKindStats extends WorkDurationStats { kind: string }
export interface WorkDayStats { date: string; session_count: number; known_count: number; total_seconds: number; average_seconds: number | null }
export interface WorkActivityDay extends WorkKindStats { date: string }
export interface AccountingDuration { nanoseconds: string; seconds: number }
export interface WorkAccounting {
  session_wall: AccountingDuration | null; occupied_wall: AccountingDuration | null; inclusive_execution: AccountingDuration;
  worker_occupancy: AccountingDuration | null; unobserved: AccountingDuration | null;
  queue_wait?: AccountingDuration;
  unobserved_intervals: { started_at: string; finished_at: string }[];
  coverage: { activity_count: number; interval_count: number; missing_start_count: number; missing_finish_count: number; clock_conflict_count: number;
    conflicted_count: number; unlinked_count: number; unknown_executor_count: number; unknown_role_count: number; source_partial: boolean; display_partial: boolean;
    capability_inventory: string; observed_adapters: string[]; outside_session_count?: number; queue_count?: number; known_queue_count?: number };
  definitions: string[];
}
export interface WorkDashboard {
  work_session_id: string | null; selected_session: WorkSummary | null; sessions: WorkSummary[]; activities: WorkSummary[];
  session_duration: WorkDurationStats; activity_kinds: WorkKindStats[]; daily: WorkDayStats[]; activity_daily: WorkActivityDay[];
  scanned_subjects: number; source_limit: number; truncated: boolean; notices: string[];
  intervals?: WaterfallNode[];
  accounting?: WorkAccounting | null; unlinked_activity_count?: number;
}
function object(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function count(value: unknown): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0; }
function seconds(value: unknown): value is number { return typeof value === "number" && Number.isFinite(value) && value >= 0; }
function metrics(value: Record<string, unknown>, sampleCount: unknown): boolean {
  return count(sampleCount) && count(value.known_count) && value.known_count <= sampleCount && seconds(value.total_seconds) &&
    (value.known_count === 0 ? value.average_seconds === null : seconds(value.average_seconds));
}
function stats(value: unknown): value is WorkDurationStats { return object(value) && metrics(value, value.count); }
function kind(value: unknown): value is WorkKindStats { return stats(value) && object(value) && typeof value.kind === "string"; }
function date(value: unknown): boolean { return typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/u.test(value); }
function day(value: unknown): value is WorkDayStats { return object(value) && date(value.date) && metrics(value, value.session_count); }
function activityDay(value: unknown): value is WorkActivityDay { return kind(value) && object(value) && date(value.date); }

function aggregateFields(value: Record<string, unknown>): boolean {
  return stats(value.session_duration) && array(value.activity_kinds, kind, 4096) && array(value.daily, day, 4096) && array(value.activity_daily, activityDay, 4096) &&
    count(value.scanned_subjects) && count(value.source_limit) && typeof value.truncated === "boolean" && array(value.notices, (item): item is string => typeof item === "string", 128);
}
function selectedSession(value: unknown, sessionId: string | undefined, summary: (item: unknown) => item is WorkSummary): boolean {
  if (value === null) return true;
  return sessionId !== undefined && summary(value) && value.subject.scope === "session" && value.subject.id === sessionId;
}
function timestamp(value: unknown): value is string { return typeof value === "string" && Number.isFinite(Date.parse(value)); }
function elapsedEvidence(value: unknown): boolean { return value === null || object(value) && typeof value.nanoseconds === "string" && /^\d+$/u.test(value.nanoseconds) && typeof value.method === "string" && (value.clock_id === null || typeof value.clock_id === "string"); }
function nodeTiming(value: Record<string, unknown>): boolean {
  return ["started_at", "finished_at"].every(field => value[field] === null || timestamp(value[field])) && elapsedEvidence(value.elapsed) &&
    (value.queued_at === undefined || value.queued_at === null || timestamp(value.queued_at)) && (value.queue_elapsed === undefined || elapsedEvidence(value.queue_elapsed));
}
function intervalNode(value: unknown): value is WaterfallNode {
  return object(value) && ["id", "title", "kind", "state"].every(field => typeof value[field] === "string") &&
    ["parent_activity_id", "executor"].every(field => value[field] === null || typeof value[field] === "string") &&
    nodeTiming(value) &&
    (value.outcome === null || ["succeeded", "failed", "canceled", "timed_out", "skipped", "infrastructure_error", "abandoned", "unknown"].includes(String(value.outcome))) &&
    ["container", "execution", "wait", "unknown"].includes(String(value.accounting_role)) &&
    array(value.conflicts, (item): item is string => typeof item === "string", 128) && array(value.gaps, (item): item is string => typeof item === "string", 128);
}
function accountingDuration(value: unknown): value is AccountingDuration { return object(value) && typeof value.nanoseconds === "string" && /^\d+$/u.test(value.nanoseconds) && seconds(value.seconds); }
function queueCoverage(value: Record<string, unknown>): boolean {
  return value.queue_count === undefined && value.known_queue_count === undefined || count(value.queue_count) && count(value.known_queue_count) && value.known_queue_count <= value.queue_count;
}
function accountingCoverage(value: unknown): value is WorkAccounting["coverage"] {
  return object(value) && ["activity_count", "interval_count", "missing_start_count", "missing_finish_count", "clock_conflict_count", "conflicted_count", "unlinked_count", "unknown_executor_count", "unknown_role_count"].every(field => count(value[field])) &&
    queueCoverage(value) && (value.outside_session_count === undefined || count(value.outside_session_count)) &&
    typeof value.source_partial === "boolean" && typeof value.display_partial === "boolean" && typeof value.capability_inventory === "string" && array(value.observed_adapters, (item): item is string => typeof item === "string", 32);
}
function accounting(value: unknown): value is WorkAccounting {
  if (!object(value) || !accountingCoverage(value.coverage)) return false;
  return ["session_wall", "occupied_wall", "worker_occupancy", "unobserved"].every(field => value[field] === null || accountingDuration(value[field])) && accountingDuration(value.inclusive_execution) &&
    (value.queue_wait === undefined || accountingDuration(value.queue_wait)) &&
    array(value.unobserved_intervals, (item): item is { started_at: string; finished_at: string } => object(item) && timestamp(item.started_at) && timestamp(item.finished_at) && Date.parse(item.finished_at) >= Date.parse(item.started_at), 4096) &&
    array(value.definitions, (item): item is string => typeof item === "string", 32);
}
function accountingFields(value: Record<string, unknown>): boolean {
  return (value.intervals === undefined || array(value.intervals, intervalNode, 4096)) &&
    (value.accounting === undefined || value.accounting === null || accounting(value.accounting)) &&
    (value.unlinked_activity_count === undefined || count(value.unlinked_activity_count));
}
export function decodeWorkDashboard(value: Record<string, unknown>, sessionId: string | undefined, summary: (item: unknown) => item is WorkSummary): WorkDashboard | undefined {
  if (value.work_session_id !== (sessionId ?? null) || !aggregateFields(value) || !selectedSession(value.selected_session, sessionId, summary)) return;
  if (!array(value.sessions, summary, 4096) || !array(value.activities, summary, 4096)) return;
  if (value.sessions.some(item => item.subject.scope !== "session") || value.activities.some(item => item.subject.scope !== "activity")) return;
  if (!accountingFields(value)) return;
  return value as unknown as WorkDashboard;
}
