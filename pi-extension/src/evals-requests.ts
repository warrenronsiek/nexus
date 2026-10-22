// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
import type { EvalHistory, EvalPage, EvalRecord, EvalsApi } from "./evals.ts";
import type { EvalContext, EvalHistoryMode, EvalIntent, EvalsState } from "./evals-state.ts";

type ReadIntent = Exclude<EvalIntent, { action: "cancel" }>;
type Observation = { action: "list"; page: EvalPage; after?: string; selectionRevision: number } |
  { action: "get"; record: EvalRecord | null; trail: EvalRecord[] } |
  { action: "why"; history: EvalHistory; context: EvalContext };
function navigationTrail(state: EvalsState, mode: EvalHistoryMode): EvalRecord[] {
  const view = state.view;
  if (view.kind === "list") return [];
  if (mode === "push") return [...view.trail, view.record];
  if (mode === "preserve") return view.trail;
  return [];
}
async function fetchObservation(api: EvalsApi, state: EvalsState, intent: ReadIntent, signal: AbortSignal): Promise<Observation> {
  switch (intent.action) {
    case "list": {
      const selectionRevision = state.selectionRevision;
      const page = await api.list(state.project, { ...state.query, ...(intent.after ? { after: intent.after } : {}), limit: 24 }, signal);
      return { action: "list", page, after: intent.after, selectionRevision };
    }
    case "get": {
      const trail = navigationTrail(state, intent.history);
      return { action: "get", record: await api.get(state.project, intent.id, signal), trail };
    }
    case "why": return { action: "why", history: await api.why(state.project, intent.context.record.id, signal), context: intent.context };
  }
}
function acceptPage(state: EvalsState, observation: Extract<Observation, { action: "list" }>): void {
  const { page, after } = observation;
  if (after && page.next_cursor === after) throw new Error("Page cursor did not advance");
  const previous = state.page;
  if (after && previous) {
    const existing = new Set(previous.records.map(record => record.id));
    state.page = { records: [...previous.records, ...page.records.filter(record => !existing.has(record.id))], next_cursor: page.next_cursor };
    if (observation.selectionRevision === state.selectionRevision && state.view.kind === "list" && state.page.records.length > previous.records.length) state.selected = previous.records.length;
  } else state.page = page;
  state.selected = Math.min(state.selected, Math.max(0, state.page.records.length - 1));
}
function acceptObservation(state: EvalsState, observation: Observation): void {
  switch (observation.action) {
    case "list": acceptPage(state, observation); break;
    case "get": {
      if (!observation.record) throw new Error("The selected record is unavailable");
      state.view = { kind: "detail", record: observation.record, trail: observation.trail, scroll: 0 }; break;
    }
    case "why": state.view = { ...observation.context, kind: "why", history: observation.history, selected: 0 }; break;
  }
}

/** Owns the one active read and its cancellation/acceptance lifecycle across every eval view. */
export class EvalsRequests {
  private pending?: AbortController;
  constructor(private readonly state: EvalsState, private readonly api: EvalsApi, private readonly changed: () => void) {}
  cancel(mode: "silent" | "report_cancellation" = "silent"): void {
    if (this.pending && mode === "report_cancellation") this.state.status = "read canceled · retained data";
    this.pending?.abort(); this.pending = undefined;
  }
  run(intent: EvalIntent): void {
    if (intent.action === "cancel") this.cancel("report_cancellation");
    else if (this.state.active) void this.read(intent);
  }
  private async read(intent: ReadIntent): Promise<void> {
    if (intent.action === "list" && intent.after && this.pending) return;
    this.cancel();
    const request = new AbortController();
    this.pending = request; this.state.status = "loading…"; this.changed();
    try {
      const observation = await fetchObservation(this.api, this.state, intent, request.signal);
      if (this.pending !== request || !this.state.active) return;
      acceptObservation(this.state, observation); this.state.status = "live";
    } catch (error) {
      if (this.pending === request) {
        const retained = this.state.page || this.state.view.kind !== "list";
        this.state.status = `${retained ? "stale" : "unavailable"} · ${error instanceof Error ? error.message : String(error)} · r retry`;
      }
    } finally {
      if (this.pending === request) { this.pending = undefined; this.changed(); }
    }
  }
}
