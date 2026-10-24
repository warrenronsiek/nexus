// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import type { WorkApi, WorkDetail, WorkPage, WorkSubject } from "./work.ts";
import { clampWork, newWorkTree, pushWorkView, workRows, type WorkState, type WorkTree } from "./work-state.ts";

export type WorkIntent = { action: "list"; after?: string } |
  { action: "children"; tree: WorkTree; parent: string | null; after?: string } |
  { action: "get"; subject: WorkSubject; presentation: "detail" | "links" | "refresh" | "session" } |
  { action: "events"; record: WorkDetail; after?: string } | { action: "refresh_tree"; tree: WorkTree } | { action: "cancel" };
type ReadIntent = Exclude<WorkIntent, { action: "cancel" }>;
type Change = () => void;
function advanced(after: string | undefined, cursor: string | null): void { if (after && cursor === after) throw new Error("Page cursor did not advance"); }
function acceptList(state: WorkState, page: WorkPage, intent: Extract<ReadIntent, { action: "list" }>, revision: number): void {
  advanced(intent.after, page.next_cursor);
  const previous = state.page;
  const existing = new Set(previous?.items.map(item => item.subject.id));
  state.page = intent.after && previous ? { items: [...previous.items, ...page.items.filter(item => !existing.has(item.subject.id))], next_cursor: page.next_cursor } : page;
  if (intent.after && previous && revision === state.selectionRevision && state.page.items.length > previous.items.length) state.selected = previous.items.length;
  state.selected = clampWork(state.selected, state.page.items.length);
}
async function list(api: WorkApi, state: WorkState, intent: Extract<ReadIntent, { action: "list" }>, signal: AbortSignal): Promise<Change> {
  const revision = state.selectionRevision;
  const load = state.listing === "sessions" ? api.list.bind(api) : api.activities.bind(api);
  const page = await load(state.project, { ...state.query, ...(intent.after ? { after: intent.after } : {}), limit: 24 }, signal);
  return () => acceptList(state, page, intent, revision);
}
async function children(api: WorkApi, state: WorkState, intent: Extract<ReadIntent, { action: "children" }>, signal: AbortSignal): Promise<Change> {
  const { tree, parent, after } = intent;
  const page = await api.timeline(state.project, tree.session.subject.id, parent, { ...(after ? { after } : {}), limit: 24 }, signal);
  return () => {
    advanced(after, page.next_cursor);
    const key = parent ?? ""; const previous = tree.pages.get(key);
    const known = new Set(previous?.items.map(item => item.subject.id));
    tree.pages.set(key, after && previous ? { ...page, items: [...previous.items, ...page.items.filter(item => !known.has(item.subject.id))], unresolved_links: [...previous.unresolved_links, ...page.unresolved_links] } : page);
    if (parent !== null) tree.expanded.add(parent);
  };
}
async function detail(api: WorkApi, state: WorkState, intent: Extract<ReadIntent, { action: "get" }>, signal: AbortSignal): Promise<Change> {
  const record = await api.get(state.project, intent.subject, signal);
  if (!record) throw new Error("The selected activity or session is unavailable");
  if (intent.presentation === "session") {
    const tree = newWorkTree(record.summary);
    tree.pages.set("", await api.timeline(state.project, record.summary.subject.id, null, { limit: 24 }, signal));
    return () => pushWorkView(state, { kind: "tree", tree });
  }
  return () => {
    if (intent.presentation === "refresh" && (state.view.kind === "detail" || state.view.kind === "links" || state.view.kind === "memberships")) state.view.record = record;
    else pushWorkView(state, intent.presentation === "links" ? { kind: "links", record, selected: 0 } : { kind: "detail", record, scroll: 0 });
  };
}
async function events(api: WorkApi, state: WorkState, intent: Extract<ReadIntent, { action: "events" }>, signal: AbortSignal): Promise<Change> {
  const previous = state.view;
  const page = await api.events(state.project, intent.record.summary.subject, { ...(intent.after ? { after: intent.after } : {}), limit: 24 }, signal);
  return () => {
    advanced(intent.after, page.next_cursor);
    if (intent.after && previous.kind === "events") {
      const known = new Set(previous.page.observations.map(item => item.id));
      previous.page = { observations: [...previous.page.observations, ...page.observations.filter(item => !known.has(item.id))], next_cursor: page.next_cursor };
    } else if (previous.kind === "events") previous.page = page;
    else pushWorkView(state, { kind: "events", record: intent.record, page, scroll: 0 });
  };
}
async function refreshTree(api: WorkApi, state: WorkState, tree: WorkTree, signal: AbortSignal): Promise<Change> {
  const keys = ["", ...tree.expanded].slice(0, 32);
  const [record, pages] = await Promise.all([
    api.get(state.project, tree.session.subject, signal),
    Promise.all(keys.map(key => api.timeline(state.project, tree.session.subject.id, key || null, { limit: 24 }, signal))),
  ]);
  if (!record) throw new Error("The selected work session is unavailable");
  return () => {
    tree.session = record.summary; tree.pages = new Map(keys.map((key, index) => [key, pages[index]!]));
    tree.expanded = new Set(keys.filter(Boolean)); tree.selected = clampWork(tree.selected, workRows(tree).length);
  };
}
function fetchChange(api: WorkApi, state: WorkState, intent: ReadIntent, signal: AbortSignal): Promise<Change> {
  switch (intent.action) {
    case "list": return list(api, state, intent, signal);
    case "children": return children(api, state, intent, signal);
    case "get": return detail(api, state, intent, signal);
    case "events": return events(api, state, intent, signal);
    case "refresh_tree": return refreshTree(api, state, intent.tree, signal);
  }
}
export class WorkRequests {
  private pending?: AbortController;
  constructor(private readonly state: WorkState, private readonly api: WorkApi, private readonly changed: () => void) {}
  cancel(): void { this.pending?.abort(); this.pending = undefined; }
  run(intent: WorkIntent): void {
    if (intent.action === "cancel") this.cancel();
    else if (this.state.active && !("after" in intent && intent.after && this.pending)) void this.read(intent);
  }
  private async read(intent: ReadIntent): Promise<void> {
    this.cancel();
    const request = new AbortController(); this.pending = request; this.state.status = "loading…"; this.changed();
    try {
      const accept = await fetchChange(this.api, this.state, intent, request.signal);
      if (this.pending !== request || !this.state.active) return;
      accept(); this.state.status = "live";
    } catch (error) {
      if (this.pending === request) this.state.status = `${this.state.page ? "stale" : "unavailable"} · ${error instanceof Error ? error.message : String(error)} · r retry`;
    } finally {
      if (this.pending === request) { this.pending = undefined; this.changed(); }
    }
  }
}
