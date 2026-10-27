// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { Key, matchesKey } from "@earendil-works/pi-tui";
import { WORK_OUTCOMES, type WorkSubject, type WorkSummary } from "./work.ts";
import { WORK_DETAIL_ROWS, clampWork, newWorkDashboard, newWorkTree, pushWorkView, resetWorkList, workRows, type WorkFilter, type WorkState, type WorkTree } from "./work-state.ts";
import type { WorkIntent } from "./work-requests.ts";
import { dashboardItems, sessionWaterfall } from "./work-dashboard.ts";
import { moveWaterfall, type WaterfallMove } from "./work-waterfall.ts";
import { editTextFilter } from "./text-filter.ts";

interface WorkInput { handled: boolean; intent?: WorkIntent }
const FILTERS: Record<string, WorkFilter> = { "/": "search", o: "repository", h: "host_session_id", k: "kind", a: "occurred_after", b: "occurred_before", v: "environment" };
const DIRECTIONS: Record<string, number> = { up: -1, down: 1, pageUp: -10, pageDown: 10, j: 1 };
export function refreshWork(state: WorkState): WorkIntent {
  const view = state.view;
  if (view.kind === "dashboard") return { action: "dashboard", view };
  if (view.kind === "list") return { action: "list" };
  if (view.kind === "tree") return { action: "refresh_tree", tree: view.tree };
  if (view.kind === "events") return { action: "events", record: view.record };
  return { action: "get", subject: view.record.summary.subject, presentation: "refresh" };
}
function applyFilter(state: WorkState): WorkIntent { resetWorkList(state); return { action: "list" }; }
function edit(state: WorkState, data: string): WorkInput {
  const applied = editTextFilter(state, data);
  if (applied === "kind") state.listing = "activities";
  return { handled: true, intent: applied ? applyFilter(state) : undefined };
}
function selectedSubject(state: WorkState): WorkSubject | undefined {
  const view = state.view;
  if (view.kind === "dashboard") return dashboardItems(view)[view.selected]?.subject;
  if (view.kind === "list") return state.page?.items[state.selected]?.subject;
  if (view.kind === "tree") { const row = workRows(view.tree)[view.tree.selected]; return row?.kind === "activity" ? row.summary.subject : undefined; }
  return view.record.summary.subject;
}
function openSession(state: WorkState, session: WorkSummary): WorkIntent {
  const tree = newWorkTree(session);
  pushWorkView(state, { kind: "tree", tree }); return { action: "children", tree, parent: null };
}
function more(state: WorkState): WorkIntent | undefined {
  const view = state.view;
  if (view.kind === "list" && state.page?.next_cursor) return { action: "list", after: state.page.next_cursor };
  if (view.kind === "events" && view.page.next_cursor) return { action: "events", record: view.record, after: view.page.next_cursor };
  if (view.kind !== "tree") return;
  const row = workRows(view.tree)[view.tree.selected];
  const parent = row?.kind === "more" ? row.parent : row?.kind === "activity" && view.tree.expanded.has(row.summary.subject.id) ? row.summary.subject.id : null;
  const page = view.tree.pages.get(parent ?? "");
  return page?.next_cursor ? { action: "children", tree: view.tree, parent, after: page.next_cursor } : undefined;
}
function selectedLink(state: WorkState): WorkIntent | undefined {
  if (state.view.kind !== "links") return;
  const target = state.view.record.links[state.view.selected]?.target;
  return target?.kind === "activity" || target?.kind === "session" ? { action: "get", subject: { scope: target.kind, id: target.id }, presentation: "detail" } : undefined;
}
function enterDashboard(state: WorkState): WorkIntent | undefined {
  const view = state.view;
  if (view.kind !== "dashboard") return;
  const selected = dashboardItems(view)[view.selected];
  if (!selected) return;
  if (view.sessionId) return { action: "get", subject: selected.subject, presentation: "detail" };
  const next = newWorkDashboard(selected.subject.id);
  pushWorkView(state, next); return { action: "dashboard", view: next };
}
function enter(state: WorkState): WorkIntent | undefined {
  const view = state.view;
  let intent: WorkIntent | undefined;
  switch (view.kind) {
    case "dashboard": intent = enterDashboard(state); break;
    case "list": {
      const selected = state.page?.items[state.selected];
      if (selected) intent = selected.subject.scope === "session" ? openSession(state, selected) : { action: "get", subject: selected.subject, presentation: "detail" };
      break;
    }
    case "memberships": {
      const id = view.record.summary.work_session_ids[view.selected];
      if (id) intent = { action: "get", subject: { scope: "session", id }, presentation: "session" };
      break;
    }
    case "tree": {
      const row = workRows(view.tree)[view.tree.selected];
      intent = row?.kind === "more" ? more(state) : row ? { action: "get", subject: row.summary.subject, presentation: "detail" } : undefined;
      break;
    }
    case "links": intent = selectedLink(state); break;
  }
  return intent;
}

function expand(state: WorkState, mode: "expand" | "collapse"): WorkIntent | undefined {
  const view = state.view;
  if (view.kind !== "tree") return;
  const tree = view.tree; const rows = workRows(tree); const row = rows[tree.selected];
  if (!row || row.kind !== "activity") return;
  const id = row.summary.subject.id;
  if (mode === "collapse") {
    state.status = "retained data";
    if (tree.expanded.has(id)) tree.expanded.delete(id);
    else { const index = rows.findIndex(item => item.kind === "activity" && item.summary.subject.id === row.summary.parent_activity_id); if (index >= 0) tree.selected = index; }
    return { action: "cancel" };
  }
  if (!row.summary.has_children) return;
  if (tree.pages.has(id)) tree.expanded.add(id); else return { action: "children", tree, parent: id };
}
function move(state: WorkState, direction: number): WorkIntent | undefined {
  const view = state.view;
  if (view.kind === "dashboard") view.selected = clampWork(view.selected + direction, dashboardItems(view).length);
  else if (view.kind === "list") {
    const length = state.page?.items.length ?? 0;
    state.selectionRevision += 1;
    if (direction === 1 && state.selected === length - 1 && state.page?.next_cursor) return more(state);
    state.selected = clampWork(state.selected + direction, length);
  } else if (view.kind === "tree") view.tree.selected = clampWork(view.tree.selected + direction, workRows(view.tree).length);
  else if (view.kind === "memberships") view.selected = clampWork(view.selected + direction, view.record.summary.work_session_ids.length);
  else if (view.kind === "links") view.selected = clampWork(view.selected + direction, view.record.links.length);
  else view.scroll = Math.max(0, Math.min(view.scroll + direction, state.detailLength - WORK_DETAIL_ROWS));
}
function cycleOutcome(state: WorkState): WorkIntent {
  const index = state.query.outcome ? WORK_OUTCOMES.indexOf(state.query.outcome) + 1 : 0;
  if (index >= WORK_OUTCOMES.length) delete state.query.outcome; else state.query.outcome = WORK_OUTCOMES[index];
  return applyFilter(state);
}
function filter(state: WorkState, key: string): WorkInput {
  if (state.view.kind !== "list") return { handled: false };
  const field = FILTERS[key];
  let intent: WorkIntent | undefined;
  if (field) state.editor = { field, text: state.query[field] ?? "" };
  else if (key === "f") {
    state.listing = state.listing === "sessions" ? "activities" : "sessions";
    if (state.listing === "sessions") delete state.query.kind;
    intent = applyFilter(state);
  }
  else if (key === "c") { state.query = {}; intent = applyFilter(state); }
  else if (key === "u") intent = cycleOutcome(state);
else return { handled: false };
  return { handled: true, intent };
}
function browseLinks(state: WorkState): WorkIntent | undefined {
  const view = state.view;
  if (view.kind === "links") return;
  if (view.kind === "detail") { pushWorkView(state, { kind: "links", record: view.record, selected: 0 }); return { action: "cancel" }; }
  const selected = selectedSubject(state);
  return selected ? { action: "get", subject: selected, presentation: "links" } : undefined;
}
function evidence(state: WorkState, key: string): WorkIntent | undefined {
  const view = state.view;
  let intent: WorkIntent | undefined;
  switch (key) {
    case "w": if ("record" in view) { pushWorkView(state, { kind: "memberships", record: view.record, selected: 0 }); intent = { action: "cancel" }; } break;
    case "s": if (view.kind === "dashboard") { const subject = view.data?.selected_session?.subject ?? selectedSubject(state); if (subject) intent = { action: "get", subject, presentation: "detail" }; } else if (view.kind === "tree") intent = { action: "get", subject: view.tree.session.subject, presentation: "detail" }; break;
    case "e": if (view.kind === "detail" || view.kind === "links") intent = { action: "events", record: view.record }; break;
    case "l": intent = browseLinks(state); break;
  }
  return intent;
}
function back(state: WorkState): WorkIntent {
  state.status = state.page ? "retained data" : "not loaded";
  state.view = state.history.pop() ?? newWorkDashboard(); return { action: "cancel" };
}
function switchPresentation(state: WorkState): WorkIntent {
  if (state.view.kind === "dashboard") { pushWorkView(state, { kind: "list" }); return { action: "list" }; }
  const view = newWorkDashboard(); pushWorkView(state, view); return { action: "dashboard", view };
}
function waterfall(state: WorkState): WorkIntent | undefined {
  const subject = state.view.kind === "tree" ? state.view.tree.session.subject : selectedSubject(state);
  if (subject?.scope !== "session") return;
  const view = newWorkDashboard(subject.id); pushWorkView(state, view);
  return { action: "dashboard", view };
}
function dashboardControl(state: WorkState, key: string): WorkInput {
  if (key === "d") return { handled: true, intent: switchPresentation(state) };
  const view = state.view;
  if (view.kind !== "dashboard") return { handled: false };
  if (view.sessionId && waterfallControl(view, key)) return { handled: true };
  switch (key) {
    case "m": view.metric = view.metric === "average" ? "total" : "average"; break;
    case "t": view.trend = view.trend === "sessions" ? "activities" : "sessions"; break;
    case "[": view.kindIndex = clampWork(view.kindIndex - 1, view.data?.activity_kinds.length ?? 0); break;
    case "]": view.kindIndex = clampWork(view.kindIndex + 1, view.data?.activity_kinds.length ?? 0); break;
    default: return { handled: false };
  }
  return { handled: true };
}
function waterfallControl(view: Extract<WorkState["view"], { kind: "dashboard" }>, key: string): boolean {
  const directions: Record<string, WaterfallMove> = { up: "up", down: "down", pageUp: "page_up", pageDown: "page_down", home: "home", end: "end", left: "left", right: "right", "+": "zoom_in", "=": "zoom_in", "-": "zoom_out" };
  const direction = directions[key];
  if (direction) { if (view.data?.selected_session) moveWaterfall(view.waterfall, sessionWaterfall(view.data), direction); return true; }
  if (key !== "j" && key !== "k") return false;
  view.selected = clampWork(view.selected + (key === "j" ? 1 : -1), dashboardItems(view).length); return true;
}
const NAVIGATION: Record<string, (state: WorkState) => WorkIntent | undefined> = {
  enter, backspace: back, escape: back, g: waterfall,
  right: state => expand(state, "expand"), space: state => expand(state, "expand"),
  left: state => state.view.kind === "tree" ? expand(state, "collapse") : back(state), n: more,
  t: state => { if (state.view.kind === "tree") state.view.tree.timeline = !state.view.tree.timeline; return undefined; },
  k: state => move(state, -1),
};
export function handleWorkInput(state: WorkState, data: string): WorkInput {
  if (state.editor) return edit(state, data);
  const keys = [Key.enter, Key.escape, Key.backspace, Key.left, Key.right, Key.up, Key.down, Key.pageUp, Key.pageDown, Key.home, Key.end, "+", "=", "-",
    Key.space, "d", "g", "m", "[", "]", "n", "t", "s", "l", "e", "j", "k", "/", "o", "h", "a", "b", "u", "v", "f", "w", "c"] as const;
  const key = data === "+" ? "+" : keys.find(key => matchesKey(data, key));
  if (key === undefined) return { handled: false };
  const dashboard = dashboardControl(state, key);
  if (dashboard.handled) return dashboard;
  if (DIRECTIONS[key]) return { handled: true, intent: move(state, DIRECTIONS[key]) };
  const filtered = filter(state, key);
  return filtered.handled ? filtered : { handled: true, intent: NAVIGATION[key] ? NAVIGATION[key](state) : evidence(state, key) };
}
