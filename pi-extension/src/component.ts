// @feature observability-ui
// @spec docs/features/observability-ui.md
import { DynamicBorder, type Theme } from "@earendil-works/pi-coding-agent";
import {
  Container,
  Key,
  matchesKey,
  ScrollView,
  SelectList,
  Text,
  type Component,
  type SelectItem,
  type TUI,
} from "@earendil-works/pi-tui";
import type { Snapshot } from "./domain.ts";
import {
  backPage,
  detailText,
  entriesFor,
  pageTitle,
  scopeLabel,
  selectEntry,
  type Entry,
  type Page,
} from "./navigation.ts";
import { SnapshotPoller, type PollingUpdate, type SnapshotLoader } from "./polling.ts";

export class NexusDashboardComponent implements Component {
  private container = new Container();
  private list: SelectList | undefined;
  private detailScroll: ScrollView | undefined;
  private page: Page = { kind: "overview" };
  private status = "live";
  private readonly poller: SnapshotPoller;

  constructor(
    private readonly tui: TUI,
    private readonly theme: Theme,
    private snapshot: Snapshot,
    load: SnapshotLoader,
    private readonly done: () => void,
  ) {
    this.poller = new SnapshotPoller(snapshot, load, (update) => this.applyPollingUpdate(update));
    this.rebuild();
    this.poller.start();
  }

  render(width: number): string[] {
    return this.container.render(width);
  }

  invalidate(): void {
    this.container.invalidate();
  }

  handleInput(data: string): void {
    if (matchesKey(data, "q")) {
      this.done();
      return;
    }
    if (matchesKey(data, "r")) {
      void this.poller.refresh(this.snapshot.dashboard.project_id, "manual");
      return;
    }
    if (this.page.kind === "detail" && (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace) || matchesKey(data, Key.left))) {
      this.goBack();
      return;
    }
    if (this.page.kind === "detail" && this.scrollDetail(data)) return;
    this.list?.handleInput(data);
    this.tui.requestRender();
  }

  dispose(): void {
    this.poller.dispose();
  }

  private applyPollingUpdate(update: PollingUpdate): void {
    this.snapshot = update.snapshot;
    this.status = update.status;
    if (update.mode === "scope" && update.status === "live") this.page = { kind: "overview" };
    this.rebuild();
    this.tui.requestRender();
  }

  private goBack(): void {
    const previous = backPage(this.page);
    if (previous === null) {
      this.done();
      return;
    }
    this.page = previous;
    this.rebuild();
    this.tui.requestRender();
  }

  private rebuild(): void {
    const selectedValue = this.list?.getSelectedItem()?.value;
    const container = new Container();
    this.addHeader(container);
    this.addBody(container, selectedValue);
    this.addFooter(container);
    this.container = container;
  }

  private addHeader(container: Container): void {
    container.addChild(new DynamicBorder((text) => this.theme.fg("accent", text)));
    container.addChild(new Text(this.theme.fg("accent", this.theme.bold(pageTitle(this.page))), 1, 0));
    container.addChild(
      new Text(
        this.theme.fg(
          "muted",
          `${scopeLabel(this.snapshot)} · ${this.status} · ${new Date(this.snapshot.dashboard.generated_at).toLocaleTimeString()}`,
        ),
        1,
        0,
      ),
    );
  }

  private addBody(container: Container, selectedValue: string | undefined): void {
    if (this.page.kind === "overview") this.addOverviewCounts(container);
    this.list = undefined;
    this.detailScroll = undefined;
    if (this.page.kind === "detail") {
      const detail = new ScrollView(new Text(detailText(this.snapshot, this.page), 1, 1), {
        primary: true,
        scrollbar: "always",
        scrollbarTrackStyle: (text) => this.theme.fg("dim", text),
        scrollbarThumbStyle: (text) => this.theme.fg("accent", text),
      });
      this.detailScroll = detail;
      container.addChild(detail);
      return;
    }
    const entries = entriesFor(this.snapshot, this.page);
    const list = this.createList(entries, selectedValue);
    this.list = list;
    container.addChild(list);
  }

  private addOverviewCounts(container: Container): void {
    const counts = this.snapshot.dashboard.counts;
    container.addChild(
      new Text(
        `${this.theme.fg("success", String(counts.active_sessions))} sessions   ${this.theme.fg("accent", String(counts.active_claims))} claims   ${this.theme.fg(counts.open_conflicts > 0 ? "error" : "muted", String(counts.open_conflicts))} conflicts`,
        1,
        1,
      ),
    );
  }

  private createList(entries: Entry[], selectedValue: string | undefined): SelectList {
    const list = new SelectList(entries satisfies SelectItem[], Math.min(entries.length, 12), {
      selectedPrefix: (text) => this.theme.fg("accent", text),
      selectedText: (text) => this.theme.fg("accent", text),
      description: (text) => this.theme.fg("muted", text),
      scrollInfo: (text) => this.theme.fg("dim", text),
      noMatch: (text) => this.theme.fg("warning", text),
    });
    const selectedIndex = entries.findIndex((entry) => entry.value === selectedValue);
    if (selectedIndex >= 0) list.setSelectedIndex(selectedIndex);
    list.onSelect = (item) => this.open(item.value);
    list.onCancel = () => this.goBack();
    return list;
  }

  private open(value: string): void {
    const selection = selectEntry(this.snapshot, this.page, value);
    if (selection.kind === "scope") {
      void this.poller.refresh(selection.projectId, "scope");
      return;
    }
    this.page = selection.page;
    this.rebuild();
    this.tui.requestRender();
  }

  private scrollDetail(data: string): boolean {
    let lines = 0;
    if (matchesKey(data, Key.up) || matchesKey(data, "k")) lines = -1;
    if (matchesKey(data, Key.down) || matchesKey(data, "j")) lines = 1;
    if (matchesKey(data, Key.pageUp)) lines = -10;
    if (matchesKey(data, Key.pageDown)) lines = 10;
    if (lines === 0) return false;
    this.detailScroll?.scrollBy(lines);
    this.tui.requestRender();
    return true;
  }

  private addFooter(container: Container): void {
    const hint = this.page.kind === "detail" ? "j/k scroll · esc/← back · r refresh · q close" : "↑↓ navigate · enter open · esc back · r refresh · q close";
    container.addChild(new Text(this.theme.fg("dim", hint), 1, 1));
    container.addChild(new DynamicBorder((text) => this.theme.fg("accent", text)));
  }
}
