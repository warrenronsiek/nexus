// @entrypoint AnalyticsBrowser
// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { matchesKey, Key } from "@earendil-works/pi-tui";
import { entries, renderAnalytics, type View, type Entry } from "./analytics-view.ts";
import { AnalyticsRequests } from "./analytics-requests.ts";
import { decodeCatalog, decodeExamples, type AnalyticsBundle, type AnalyticsApi, type Catalog, type Examples, type ExampleFilter } from "./analytics-data.ts";
export { decodeAnalytics } from "./analytics-data.ts";
export type { AnalyticsApi, AnalyticsBundle } from "./analytics-data.ts";

export class AnalyticsBrowser {
  private data?: AnalyticsBundle;
  private catalog?: Catalog;
  private project: string | null = null;
  private view: View = "activities";
  private selected = 0;
  private detail?: Entry;
  private examples?: Examples;
  private offset = 0;
  private scroll = 0;
  private catalogOffset = 0;
  private readonly requests: AnalyticsRequests;
  private maximumScroll = 0;
  constructor(private readonly api: AnalyticsApi, private readonly updated: () => void) { this.requests = new AnalyticsRequests(updated); }
  get legacy(): boolean { return this.view === "legacy"; }
  update(data: AnalyticsBundle, project: string | null): void {
    if (project !== this.project) this.clear();
    this.project = project;
    this.data = data;
    if (this.catalogOffset === 0) this.catalog = data.catalog;
  }
  clear(): void { this.deactivate(); this.data = undefined; this.catalog = undefined; this.detail = undefined; this.examples = undefined; this.catalogOffset = 0; this.selected = 0; }
  deactivate(): void { this.requests.cancel(); }
  render(width: number): string[] {
    if (!this.data || !this.catalog) return ["Waiting for activity analytics…"];
    const rendered = renderAnalytics({ data: this.data, catalog: this.catalog, view: this.view, selected: this.selected, detail: this.detail, examples: this.examples, scroll: this.scroll, page: Math.floor(this.catalogOffset / 32) + 1, status: this.requests.status }, width);
    this.maximumScroll = rendered.maximumScroll;
    return rendered.lines;
  }
  handleInput(input: string): boolean {
    const views: Record<string, View> = { "1": "activities", "2": "capabilities", "3": "opportunities", "4": "legacy" };
    if (views[input]) { this.deactivate(); this.view = views[input]; this.detail = undefined; this.selected = 0; this.updated(); return true; }
    if (this.legacy || !this.data || !this.catalog) return false;
    if (this.detail && matchesKey(input, Key.escape)) { this.deactivate(); this.detail = undefined; this.updated(); return true; }
    let handled = true;
    const direction = directionFor(input);
    if (direction) this.move(direction);
    else if (input === "n" || input === "p") this.page(input === "n" ? 1 : -1);
    else if (matchesKey(input, Key.enter)) this.open();
    else handled = false;
    return handled;
  }
  private move(direction: number): void {
    if (this.detail) this.scroll = Math.max(0, Math.min(this.scroll + direction, this.maximumScroll));
    else if (this.data && this.catalog) this.selected = Math.max(0, Math.min(this.selected + direction, entries(this.data, this.catalog, this.view).length - 1));
    this.updated();
  }
  private open(): void {
    if (!this.data || !this.catalog) return;
    this.detail = entries(this.data, this.catalog, this.view)[this.selected];
    this.examples = undefined; this.offset = 0; this.scroll = 0;
    if (this.detail) this.loadExamples(0);
  }
  private page(direction: number): void {
    if (this.detail) {
      const next = direction > 0 ? this.examples?.next_offset : Math.max(0, this.offset - 32);
      if (next !== null && next !== undefined) this.loadExamples(next);
    } else if (this.view === "capabilities") {
      const next = direction > 0 ? this.catalog?.next_offset : Math.max(0, this.catalogOffset - 32);
      if (next !== null && next !== undefined) this.loadCatalog(next);
    }
  }
  refresh(): void { if (this.detail) this.loadExamples(this.offset); else if (this.catalogOffset > 0) this.loadCatalog(this.catalogOffset); }
  private loadExamples(offset: number): void {
    if (!this.detail) return;
    const filter = { ...this.detail.filter, offset };
    this.requests.run(
      (signal) => this.api.examples(this.project, filter, signal),
      (value) => {
        this.examples = decodeExamples(value);
        this.offset = offset;
        this.scroll = 0;
      },
    );
  }
  private loadCatalog(offset: number): void {
    this.requests.run(
      (signal) => this.api.catalog(this.project, offset, signal),
      (value) => {
        this.catalog = decodeCatalog(value);
        this.catalogOffset = offset;
        this.selected = 0;
      },
    );
  }
}

function directionFor(input: string): number {
    if (input === "j" || matchesKey(input, Key.down)) return 1;
    if (input === "k" || matchesKey(input, Key.up)) return -1;
    return 0;
  }
