// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
import type { ConcernSessionTitle, ConcernsApi } from "./concerns.ts";
import type { ConcernIntent, ConcernsState } from "./concerns-state.ts";

type ReadIntent = Exclude<ConcernIntent, { action: "cancel" }>;
type Change = () => void;
function advanced(after: string | undefined, cursor: string | null): void { if (after && after === cursor) throw new Error("Concern page cursor did not advance"); }
function retained<T>(previous: T[] | undefined, page: T[], selected: number, identify: (item: T) => string, append: boolean): { items: T[]; selected: number } {
  const id = previous?.[selected] === undefined ? undefined : identify(previous[selected]!);
  const known = new Set(previous?.map(identify));
  const items = append && previous ? [...previous, ...page.filter(item => !known.has(identify(item)))] : page;
  return { items, selected: Math.max(0, items.findIndex(item => identify(item) === id)) };
}
function mergedTitles(previous: ConcernSessionTitle[], page: ConcernSessionTitle[]): ConcernSessionTitle[] {
  const key = (title: ConcernSessionTitle) => JSON.stringify([title.agent, title.host_session_id, title.work_session_id]);
  const titles = new Map(previous.map(title => [key(title), title]));
  for (const title of page) titles.set(key(title), title);
  return [...titles.values()];
}
async function dashboard(api: ConcernsApi, state: ConcernsState, intent: Extract<ReadIntent, { action: "dashboard" }>, signal: AbortSignal): Promise<Change> {
  const page = await api.dashboard(state.project, { ...state.query, ...(intent.after ? { after: intent.after } : {}), limit: 24 }, signal);
  return () => {
    advanced(intent.after, page.next_cursor);
    const previous = state.dashboard, result = retained(previous?.areas, page.areas, state.selected, item => item.area.id, !!intent.after);
    state.dashboard = { ...page, areas: result.items, partial: page.partial || !!intent.after && !!previous?.partial }; state.selected = result.selected;
  };
}
async function list(api: ConcernsApi, state: ConcernsState, intent: Extract<ReadIntent, { action: "list" }>, signal: AbortSignal): Promise<Change> {
  const page = await api.list(state.project, { ...state.query, ...(intent.view.area ? { area_id: intent.view.area.area.id } : {}), ...(intent.after ? { after: intent.after } : {}), limit: 24 }, signal);
  return () => {
    advanced(intent.after, page.next_cursor);
    const previous = intent.view.page, result = retained(previous?.items, page.items, intent.view.selected, item => item.id, !!intent.after);
    intent.view.page = { ...page, items: result.items, partial: page.partial || !!intent.after && !!previous?.partial }; intent.view.selected = result.selected;
  };
}
async function get(api: ConcernsApi, state: ConcernsState, intent: Extract<ReadIntent, { action: "get" }>, signal: AbortSignal): Promise<Change> {
  const detail = await api.get(state.project, intent.concern, signal); if (!detail) throw new Error("Selected concern is unavailable");
  return () => {
    if (state.view.kind === "detail" && state.view.detail.summary.id === detail.summary.id) {
      const selected = state.view.detail.history[state.view.selected]?.input.id;
      state.view.detail = detail; state.view.selected = Math.max(0, detail.history.findIndex(item => item.input.id === selected));
    } else { state.history.push(state.view); state.view = { kind: "detail", detail, selected: 0 }; }
  };
}
async function history(api: ConcernsApi, state: ConcernsState, intent: Extract<ReadIntent, { action: "history" }>, signal: AbortSignal): Promise<Change> {
  const after = intent.view.detail.next_cursor ?? undefined;
  const page = await api.history(state.project, intent.view.detail.summary.id, after, signal);
  return () => {
    advanced(after, page.next_cursor);
    intent.view.detail.history = retained(intent.view.detail.history, page.observations, intent.view.selected, item => item.input.id, true).items;
    intent.view.detail.next_cursor = page.next_cursor;
    intent.view.detail.session_titles = mergedTitles(intent.view.detail.session_titles ?? [], page.session_titles ?? []);
  };
}
function fetchChange(api: ConcernsApi, state: ConcernsState, intent: ReadIntent, signal: AbortSignal): Promise<Change> {
  switch (intent.action) {
    case "dashboard": return dashboard(api, state, intent, signal);
    case "list": return list(api, state, intent, signal);
    case "get": return get(api, state, intent, signal);
    case "history": return history(api, state, intent, signal);
  }
}
export class ConcernsRequests {
  private pending?: AbortController;
  constructor(private readonly state: ConcernsState, private readonly api: ConcernsApi, private readonly changed: () => void) {}
  cancel(): void { this.pending?.abort(); this.pending = undefined; }
  run(intent: ConcernIntent): void { if (intent.action === "cancel") this.cancel(); else if (this.state.active) void this.read(intent); }
  private async read(intent: ReadIntent): Promise<void> {
    this.cancel(); const request = new AbortController(); this.pending = request; this.state.status = "loading…"; this.changed();
    try {
      const accept = await fetchChange(this.api, this.state, intent, request.signal);
      if (this.pending !== request || !this.state.active) return;
      accept(); this.state.status = "live";
    } catch (error) {
      if (this.pending === request) this.state.status = `${this.state.dashboard ? "stale" : "unavailable"} · ${error instanceof Error ? error.message : String(error)} · r retry`;
    } finally { if (this.pending === request) { this.pending = undefined; this.changed(); } }
  }
}
