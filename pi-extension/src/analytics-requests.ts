// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// One request owner cancels superseded page/detail reads and preserves successful data.
export class AnalyticsRequests {
  private controller?: AbortController;
  status = "";
  constructor(private readonly updated: () => void) {}
  cancel(): void { this.controller?.abort(); this.controller = undefined; this.status = ""; }
  run<T>(load: (signal: AbortSignal) => Promise<T>, apply: (value: T) => void): void {
    this.cancel();
    const controller = new AbortController(); this.controller = controller;
    this.status = "Loading…"; this.updated();
    void load(controller.signal).then((value) => {
      if (this.controller !== controller) return;
      apply(value); this.status = "";
    }).catch(() => { if (this.controller === controller) this.status = "Request failed; previous data retained. Press r to retry."; })
      .finally(() => { if (this.controller === controller) this.updated(); });
  }
}
