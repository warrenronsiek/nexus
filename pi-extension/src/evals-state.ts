// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
import { Key, matchesKey } from "@earendil-works/pi-tui";
import { EVAL_DETAIL_ROWS, EVAL_KINDS, type EvalHistory, type EvalPage, type EvalQuery, type EvalRecord } from "./evals.ts";
import { editTextFilter } from "./text-filter.ts";

export interface EvalContext { record: EvalRecord; trail: EvalRecord[] }
export type EvalView = { kind: "list" } |
  (EvalContext & { kind: "detail"; scroll: number }) |
  (EvalContext & { kind: "why"; history: EvalHistory; selected: number }) |
  (EvalContext & { kind: "links"; selected: number });
type FilterField = "search" | "repository" | "model";
export interface EvalsState {
  project: string | null; active: boolean; status: string; page?: EvalPage;
  selected: number; selectionRevision: number; detailLength: number;
  view: EvalView; query: EvalQuery; editor?: { field: FilterField; text: string };
}
export type EvalHistoryMode = "push" | "preserve" | "reset";
export type EvalIntent = { action: "list"; after?: string } | { action: "get"; id: string; history: EvalHistoryMode } |
  { action: "why"; context: EvalContext } | { action: "cancel" };
interface EvalInput { handled: boolean; intent?: EvalIntent }
type Navigation = "back" | "enter" | "why" | "links";
const NAVIGATION: Record<string, Navigation> = { escape: "back", backspace: "back", left: "back", enter: "enter", w: "why", l: "links" };
const DIRECTIONS: Record<string, number> = { up: -1, k: -1, down: 1, j: 1, pageUp: -10, pageDown: 10 };
const FILTERS: Record<string, FilterField> = { "/": "search", o: "repository", m: "model" };
const KEYS = [Key.escape, Key.backspace, Key.left, Key.enter, Key.up, Key.down, Key.pageUp, Key.pageDown, "w", "l", "n", "t", "/", "o", "m", "c", "j", "k"] as const;

export function initialEvalsState(): EvalsState {
  return { project: null, active: false, status: "not loaded", selected: 0, selectionRevision: 0, detailLength: 0, view: { kind: "list" }, query: {} };
}
export function refreshEvals(state: EvalsState): EvalIntent {
  if (state.view.kind === "detail") return { action: "get", id: state.view.record.id, history: "preserve" };
  if (state.view.kind === "why") return { action: "why", context: state.view };
  return { action: "list" };
}
export function resetEvalList(state: EvalsState): EvalIntent {
  state.page = undefined; state.selected = 0; state.view = { kind: "list" };
  return { action: "list" };
}
function editFilter(state: EvalsState, data: string): EvalInput {
  return { handled: true, intent: editTextFilter(state, data) ? resetEvalList(state) : undefined };
}
function filterCommand(state: EvalsState, key: string): EvalInput {
  if (state.view.kind !== "list") return { handled: false };
  const field = FILTERS[key];
  if (field) { state.editor = { field, text: state.query[field] ?? "" }; return { handled: true }; }
  if (key === "n") return { handled: true, intent: state.page?.next_cursor ? { action: "list", after: state.page.next_cursor } : undefined };
  if (key === "c") state.query = {};
  else if (key === "t") {
    const index = state.query.kind ? EVAL_KINDS.indexOf(state.query.kind) + 1 : 0;
    if (index >= EVAL_KINDS.length) delete state.query.kind; else state.query.kind = EVAL_KINDS[index];
  } else return { handled: false };
  return { handled: true, intent: resetEvalList(state) };
}
function back(state: EvalsState): void {
  const view = state.view;
  if (view.kind === "why" || view.kind === "links") state.view = { kind: "detail", record: view.record, trail: view.trail, scroll: 0 };
  else if (view.kind === "detail" && view.trail.length) {
    state.view = { kind: "detail", record: view.trail.at(-1)!, trail: view.trail.slice(0, -1), scroll: 0 };
  } else state.view = { kind: "list" };
}
function selectedRecord(state: EvalsState): string | undefined {
  const view = state.view;
  if (view.kind === "list") return state.page?.records[state.selected]?.id;
  if (view.kind === "links") return view.record.links[view.selected]?.target_id;
  if (view.kind === "why") return view.history.records[view.selected]?.id;
  return undefined;
}
function navigate(state: EvalsState, command: Navigation): EvalInput {
  const view = state.view;
  let intent: EvalIntent | undefined;
  if (command === "back") { back(state); intent = { action: "cancel" }; }
  else if (command === "enter") {
    const id = selectedRecord(state);
    if (id) intent = { action: "get", id, history: view.kind === "list" ? "reset" : "push" };
  } else if (view.kind !== "list") {
    if (command === "why") intent = { action: "why", context: view };
    else { state.view = { kind: "links", record: view.record, trail: view.trail, selected: 0 }; intent = { action: "cancel" }; }
  }
  return { handled: true, intent };
}
function clamp(value: number, last: number): number { return Math.max(0, Math.min(value, last)); }
function move(state: EvalsState, direction: number): EvalInput {
  const view = state.view;
  let intent: EvalIntent | undefined;
  switch (view.kind) {
    case "detail": view.scroll = clamp(view.scroll + direction, state.detailLength - EVAL_DETAIL_ROWS); break;
    case "why": view.selected = clamp(view.selected + direction, view.history.records.length - 1); break;
    case "links": view.selected = clamp(view.selected + direction, view.record.links.length - 1); break;
    case "list": {
      const last = (state.page?.records.length ?? 1) - 1;
      if (direction === 1 && state.selected === last && state.page?.next_cursor) intent = { action: "list", after: state.page.next_cursor };
      else state.selected = clamp(state.selected + direction, last);
      state.selectionRevision += 1;
    }
  }
  return { handled: true, intent };
}
export function handleEvalInput(state: EvalsState, data: string): EvalInput {
  if (state.editor) return editFilter(state, data);
  const key = KEYS.find(key => matchesKey(data, key));
  if (!key) return { handled: false };
  if (NAVIGATION[key]) return navigate(state, NAVIGATION[key]);
  if (DIRECTIONS[key]) return move(state, DIRECTIONS[key]);
  return filterCommand(state, key);
}
