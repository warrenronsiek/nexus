// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
// @entrypoint EvalsPane
import type { Component, TUI } from "@earendil-works/pi-tui";
import { sanitizeDisplayText } from "./display.ts";
import type { EvalsApi } from "./evals.ts";
import { EvalsRequests } from "./evals-requests.ts";
import { handleEvalInput, initialEvalsState, refreshEvals } from "./evals-state.ts";
import { renderEvals } from "./evals-view.ts";
import type { DashboardTheme } from "./theme.ts";

/** Shared component: navigation, transport lifecycle, and presentation have distinct owners. */
export class EvalsPane implements Component {
  private readonly state = initialEvalsState();
  private readonly requests: EvalsRequests;
  constructor(private readonly tui: TUI, private readonly theme: DashboardTheme, api: EvalsApi, private readonly updated: () => void) {
    this.requests = new EvalsRequests(this.state, api, () => this.changed());
  }
  activate(project: string | null): void {
    if (project !== this.state.project) {
      this.deactivate(); this.state.page = undefined; this.state.selected = 0; this.state.view = { kind: "list" };
    }
    this.state.project = project; this.state.active = true; this.refresh();
  }
  deactivate(): void { this.state.active = false; this.state.editor = undefined; this.requests.cancel(); }
  dispose(): void { this.deactivate(); }
  invalidate(): void {}
  statusLine(): string { return sanitizeDisplayText(`Eval history · ${this.state.project ?? "current repository"} · ${this.state.status} · read-only`); }
  refresh(): void { this.requests.run(refreshEvals(this.state)); }
  render(width: number): string[] {
    const rendered = renderEvals(this.state, width, this.theme);
    this.state.detailLength = rendered.detailLength;
    return rendered.lines;
  }
  handleInput(data: string): boolean {
    const result = handleEvalInput(this.state, data);
    if (result.intent) this.requests.run(result.intent);
    if (result.handled) this.changed();
    return result.handled;
  }
  private changed(): void { this.updated(); this.tui.requestRender(); }
}
