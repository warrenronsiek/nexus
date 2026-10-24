// @feature observability-ui
// @spec docs/features/observability-ui.md
import type { Component, TUI } from "@earendil-works/pi-tui";
import { sanitizeDisplayText } from "./display.ts";
import type { DashboardTheme } from "./theme.ts";

interface HistoryState {
  project: string | null; active: boolean; status: string; detailLength: number;
  editor?: { field: string; text: string };
}
interface HistoryRequests<Intent> { run(intent: Intent): void; cancel(): void }
interface HistoryDefinition<State, Intent, Api> {
  title: string;
  initial: () => State;
  requests: new (state: State, api: Api, changed: () => void) => HistoryRequests<Intent>;
  reset: (state: State) => void;
  refresh: (state: State) => Intent;
  input: (state: State, data: string) => { handled: boolean; intent?: Intent };
  render: (state: State, width: number, theme: DashboardTheme) => { lines: string[]; detailLength: number };
}
export interface HistoryPane extends Component {
  activate(project: string | null): void; deactivate(): void; dispose(): void;
  refresh(): void; statusLine(): string; handleInput(data: string): boolean;
}

/** One scoped component lifecycle; each history owns its typed navigation and request policies. */
class ScopedHistoryPane<State extends HistoryState, Intent, Api> implements HistoryPane {
  private readonly state: State;
  private readonly requests: HistoryRequests<Intent>;
  constructor(private readonly definition: HistoryDefinition<State, Intent, Api>, private readonly tui: TUI, private readonly theme: DashboardTheme, api: Api, private readonly updated: () => void) {
    this.state = definition.initial();
    this.requests = new definition.requests(this.state, api, () => this.changed());
  }
  activate(project: string | null): void {
    if (project !== this.state.project) { this.deactivate(); this.definition.reset(this.state); }
    this.state.project = project; this.state.active = true; this.refresh();
  }
  deactivate(): void { this.state.active = false; this.state.editor = undefined; this.requests.cancel(); }
  dispose(): void { this.deactivate(); }
  invalidate(): void {}
  statusLine(): string { return sanitizeDisplayText(`${this.definition.title} · ${this.state.project ?? "current repository"} · ${this.state.status} · read-only`); }
  refresh(): void { this.requests.run(this.definition.refresh(this.state)); }
  render(width: number): string[] {
    const result = this.definition.render(this.state, width, this.theme);
    this.state.detailLength = result.detailLength;
    return result.lines;
  }
  handleInput(data: string): boolean {
    const result = this.definition.input(this.state, data);
    if (result.intent) this.requests.run(result.intent);
    if (result.handled) this.changed();
    return result.handled;
  }
  private changed(): void { this.updated(); this.tui.requestRender(); }
}

/** Bind a history definition once while keeping each mounted component's state independent. */
export function defineHistoryPane<State extends HistoryState, Intent, Api>(definition: HistoryDefinition<State, Intent, Api>) {
  return class extends ScopedHistoryPane<State, Intent, Api> {
    constructor(tui: TUI, theme: DashboardTheme, api: Api, updated: () => void) { super(definition, tui, theme, api, updated); }
  };
}
