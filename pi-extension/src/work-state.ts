// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import type { WorkDashboard } from "./work-dashboard-wire.ts";
import type { WorkDetail, WorkEvents, WorkPage, WorkQuery, WorkSummary, WorkTimeline } from "./work.ts";

export interface WorkTree { session: WorkSummary; pages: Map<string, WorkTimeline>; expanded: Set<string>; selected: number; timeline: boolean }
export interface WorkDashboardView { kind: "dashboard"; sessionId?: string; data?: WorkDashboard; selected: number; kindIndex: number; metric: "average" | "total"; trend: "sessions" | "activities" }
export type WorkView = WorkDashboardView | { kind: "list" } | { kind: "tree"; tree: WorkTree } |
  { kind: "detail"; record: WorkDetail; scroll: number } | { kind: "links"; record: WorkDetail; selected: number } | { kind: "memberships"; record: WorkDetail; selected: number } |
  { kind: "events"; record: WorkDetail; page: WorkEvents; scroll: number };
export type WorkFilter = "search" | "repository" | "host_session_id" | "kind" | "occurred_after" | "occurred_before" | "environment";
export interface WorkState {
  listing: "sessions" | "activities"; project: string | null; active: boolean; status: string; page?: WorkPage; selected: number; selectionRevision: number;
  query: WorkQuery; editor?: { field: WorkFilter; text: string }; view: WorkView; history: WorkView[]; detailLength: number;
}
export type WorkRow = { kind: "activity"; summary: WorkSummary; depth: number } | { kind: "more"; parent: string | null; depth: number };
export const WORK_DETAIL_ROWS = 18;
export function initialWorkState(): WorkState {
  return { listing: "sessions", project: null, active: false, status: "not loaded", selected: 0, selectionRevision: 0, query: {}, view: newWorkDashboard(), history: [], detailLength: 0 };
}
export function resetWorkList(state: WorkState): void {
  state.page = undefined; state.selected = 0; state.view = { kind: "list" }; state.history = [];
}
/** Traverse only explicit expanded ownership; defensive path guards keep conflicted graphs navigable. */
export function workRows(tree: WorkTree): WorkRow[] {
  const rows: WorkRow[] = [];
  function children(parent: string | null, depth: number, path: Set<string>): void {
    if (depth > 32 || rows.length >= 2048) return;
    const page = tree.pages.get(parent ?? "");
    const items = page?.items ?? [];
    for (let index = 0; index < items.length && rows.length < 2048; index++) {
      const summary = items[index]!;
      rows.push({ kind: "activity", summary, depth });
      if (tree.expanded.has(summary.subject.id) && !path.has(summary.subject.id)) children(summary.subject.id, depth + 1, new Set([...path, summary.subject.id]));
    }
    if (page?.next_cursor) rows.push({ kind: "more", parent, depth });
  }
  children(null, 0, new Set());
  return rows;
}
export function pushWorkView(state: WorkState, next: WorkView): void { state.history.push(state.view); state.view = next; }
export function clampWork(value: number, length: number): number { return Math.max(0, Math.min(value, Math.max(0, length - 1))); }

export function newWorkTree(session: WorkSummary): WorkTree { return { session, pages: new Map(), expanded: new Set(), selected: 0, timeline: false }; }

export function newWorkDashboard(sessionId?: string): WorkDashboardView { return { kind: "dashboard", sessionId, selected: 0, kindIndex: 0, metric: "average", trend: "sessions" }; }
export function resetWorkScope(state: WorkState): void { resetWorkList(state); state.view = newWorkDashboard(); }
