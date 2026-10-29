// @feature observability-ui
// @spec docs/features/observability-ui.md
import { matchesKey, Text, type Component, type TUI } from "@earendil-works/pi-tui";
import { NexusDashboardComponent, type DashboardOptions } from "./component.ts";
import { fitDisplayText } from "./display.ts";
import type { DashboardTheme } from "./theme.ts";

/** Own startup reads without making service availability a condition of opening the terminal. */
export class NexusDashboardConnection implements Component {
  private dashboard: NexusDashboardComponent | undefined;
  private controller: AbortController | undefined;
  private timer: NodeJS.Timeout | undefined;
  private error: string | undefined;
  private disposed = false;

  constructor(
    private readonly tui: TUI,
    private readonly theme: DashboardTheme,
    private readonly options: DashboardOptions,
  ) { void this.connect(); }

  render(width: number): string[] {
    if (this.dashboard) return this.dashboard.render(width);
    const message = this.error === undefined ? "Connecting to Nexus…" : `Unavailable: ${this.error}`;
    const hint = this.error === undefined ? "q close" : "Retrying in 2s · r retry now · q close";
    return new Text([
      this.theme.bold("Nexus"), "", fitDisplayText(message, width - 2), "", hint,
    ].join("\n"), 1, 1).render(width);
  }

  invalidate(): void { this.dashboard?.invalidate(); }

  handleInput(data: string): void {
    if (this.dashboard) { this.dashboard.handleInput?.(data); return; }
    if (matchesKey(data, "q") || matchesKey(data, "escape")) {
      this.options.done();
    } else if (matchesKey(data, "r")) {
      void this.connect();
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.clearTimer();
    this.controller?.abort();
    this.dashboard?.dispose();
  }

  private async connect(): Promise<void> {
    if (this.disposed || this.controller !== undefined || this.dashboard !== undefined) return;
    this.clearTimer();
    const controller = new AbortController();
    this.controller = controller;
    this.error = undefined;
    this.tui.requestRender();
    try {
      const snapshot = await this.options.loadSnapshot(null, controller.signal);
      if (this.disposed || controller.signal.aborted) return;
      this.dashboard = new NexusDashboardComponent(this.tui, this.theme, snapshot, this.options);
    } catch (error) {
      if (this.disposed || controller.signal.aborted) return;
      controller.abort();
      this.error = error instanceof Error ? error.message : String(error);
      this.timer = setTimeout(() => { void this.connect(); }, 2_000);
      this.timer.unref();
    } finally {
      this.controller = undefined;
      if (!this.disposed) this.tui.requestRender();
    }
  }

  private clearTimer(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
