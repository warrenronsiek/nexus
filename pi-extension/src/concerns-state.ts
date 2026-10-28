// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
import { Key, matchesKey } from "@earendil-works/pi-tui";
import type { ConcernAreaSummary, ConcernDashboard, ConcernDetail, ConcernPage, ConcernQuery, ConcernSort } from "./concerns.ts";

export type ConcernView = { kind: "overview" } | { kind: "area"; area: ConcernAreaSummary | null; page?: ConcernPage; selected: number } | { kind: "detail"; detail: ConcernDetail; selected: number };
export interface ConcernsState { project: string | null; active: boolean; status: string; detailLength: number; dashboard?: ConcernDashboard; selected: number; query: ConcernQuery; view: ConcernView; history: ConcernView[] }
export type ConcernIntent = { action: "dashboard"; after?: string } | { action: "list"; view: Extract<ConcernView, { kind: "area" }>; after?: string } | { action: "get"; concern: string } | { action: "history"; view: Extract<ConcernView, { kind: "detail" }> } | { action: "cancel" };
export function initialConcernsState(): ConcernsState { return { project: null, active: false, status: "not loaded", detailLength: 0, selected: 0, query: { sort: "work_sessions" }, view: { kind: "overview" }, history: [] }; }
export function resetConcerns(state: ConcernsState): void { Object.assign(state, initialConcernsState(), { project: state.project, active: state.active }); }
export function refreshConcerns(state: ConcernsState): ConcernIntent {
  if (state.view.kind === "area") return { action: "list", view: state.view };
  if (state.view.kind === "detail") return { action: "get", concern: state.view.detail.summary.id };
  return { action: "dashboard" };
}
function more(state: ConcernsState): ConcernIntent | undefined {
  if (state.view.kind === "area") return state.view.page?.next_cursor ? { action: "list", view: state.view, after: state.view.page.next_cursor } : undefined;
  if (state.view.kind === "detail") return state.view.detail.next_cursor ? { action: "history", view: state.view } : undefined;
  return state.dashboard?.next_cursor ? { action: "dashboard", after: state.dashboard.next_cursor } : undefined;
}
function enter(state: ConcernsState): ConcernIntent | undefined {
  if (state.view.kind === "overview") {
    const area = state.dashboard?.areas[state.selected]; if (!area) return;
    const view: Extract<ConcernView, { kind: "area" }> = { kind: "area", area, selected: 0 };
    state.history.push(state.view); state.view = view; return { action: "list", view };
  }
  if (state.view.kind === "area") {
    const concern = state.view.page?.items[state.view.selected];
    return concern ? { action: "get", concern: concern.id } : undefined;
  }
}
function allConcerns(state: ConcernsState): ConcernIntent | undefined {
  if (state.view.kind !== "overview") return;
  const view: Extract<ConcernView, { kind: "area" }> = { kind: "area", area: null, selected: 0 };
  state.history.push(state.view); state.view = view; return { action: "list", view };
}
function move(state: ConcernsState, delta: number): void {
  const view = state.view;
  const length = view.kind === "overview" ? state.dashboard?.areas.length ?? 0 : view.kind === "area" ? view.page?.items.length ?? 0 : view.detail.history.length;
  if (view.kind === "overview") state.selected = Math.max(0, Math.min(state.selected + delta, length - 1));
  else view.selected = Math.max(0, Math.min(view.selected + delta, length - 1));
}
function sort(state: ConcernsState): ConcernIntent {
  const sorts: ConcernSort[] = ["work_sessions", "recent", "returns"];
  state.query.sort = sorts[(sorts.indexOf(state.query.sort ?? "work_sessions") + 1) % sorts.length];
  state.selected = 0; state.dashboard = undefined; return { action: "dashboard" };
}
export function handleConcernInput(state: ConcernsState, data: string): { handled: boolean; intent?: ConcernIntent } {
  const key = ([Key.enter, Key.escape, Key.backspace, Key.left, Key.up, Key.down, Key.pageUp, Key.pageDown, "j", "k", "n", "m", "a"] as const).find(key => matchesKey(data, key));
  if (!key) return { handled: false };
  let intent: ConcernIntent | undefined;
  switch (key) {
    case Key.enter: intent = enter(state); break;
    case "n": intent = more(state); break;
    case "a": intent = allConcerns(state); break;
    case "m": if (state.view.kind === "overview") intent = sort(state); break;
    case Key.escape: case Key.backspace: case Key.left:
      state.view = state.history.pop() ?? { kind: "overview" }; intent = { action: "cancel" }; break;
    default: move(state, ({ up: -1, k: -1, pageUp: -6, pageDown: 6 } as Record<string, number>)[key] ?? 1);
  }
  return { handled: true, intent };
}
