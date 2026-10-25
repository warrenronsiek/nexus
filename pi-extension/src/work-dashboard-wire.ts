// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
// @boundary work-dashboard-wire: validate scoped aggregate projections before display; exact source evidence remains in work detail.
import type { WorkSummary } from "./work.ts";

export interface WorkDurationStats { count: number; known_count: number; total_seconds: number; average_seconds: number | null }
export interface WorkKindStats extends WorkDurationStats { kind: string }
export interface WorkDayStats { date: string; session_count: number; known_count: number; total_seconds: number; average_seconds: number | null }
export interface WorkActivityDay extends WorkKindStats { date: string }
export interface WorkDashboard {
  work_session_id: string | null; selected_session: WorkSummary | null; sessions: WorkSummary[]; activities: WorkSummary[];
  session_duration: WorkDurationStats; activity_kinds: WorkKindStats[]; daily: WorkDayStats[]; activity_daily: WorkActivityDay[];
  scanned_subjects: number; source_limit: number; truncated: boolean; notices: string[];
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
function array<T>(value: unknown, check: (item: unknown) => item is T, maximum: number): value is T[] { return Array.isArray(value) && value.length <= maximum && value.every(check); }

function aggregateFields(value: Record<string, unknown>): boolean {
  return stats(value.session_duration) && array(value.activity_kinds, kind, 4096) && array(value.daily, day, 4096) && array(value.activity_daily, activityDay, 4096) &&
    count(value.scanned_subjects) && count(value.source_limit) && typeof value.truncated === "boolean" && array(value.notices, (item): item is string => typeof item === "string", 128);
}
function selectedSession(value: unknown, sessionId: string | undefined, summary: (item: unknown) => item is WorkSummary): boolean {
  if (value === null) return true;
  return sessionId !== undefined && summary(value) && value.subject.scope === "session" && value.subject.id === sessionId;
}
export function decodeWorkDashboard(value: Record<string, unknown>, sessionId: string | undefined, summary: (item: unknown) => item is WorkSummary): WorkDashboard | undefined {
  if (value.work_session_id !== (sessionId ?? null) || !aggregateFields(value) || !selectedSession(value.selected_session, sessionId, summary)) return;
  if (!array(value.sessions, summary, 4096) || !array(value.activities, summary, 4096)) return;
  if (value.sessions.some(item => item.subject.scope !== "session") || value.activities.some(item => item.subject.scope !== "activity")) return;
  return value as unknown as WorkDashboard;
}
