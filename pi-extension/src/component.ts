// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
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
import {
  SnapshotPoller,
  type PollingUpdate,
  type SnapshotLoader,
  type UsageLoader,
} from "./polling.ts";
import {
  nextDashboardTab,
  renderTabHeader,
  UsagePane,
  type DashboardTab,
  type UsageTab,
} from "./usage-pane.ts";

export { nextDashboardTab } from "./usage-pane.ts";

interface DashboardOptions {
  loadSnapshot: SnapshotLoader;
  loadUsage: UsageLoader;
  done: () => void;
}

type DashboardCommand = "quit" | "next-tab" | "previous-tab" | "refresh" | "coordination";

function dashboardCommand(data: string, tab: DashboardTab): DashboardCommand | undefined {
  let command: DashboardCommand | undefined;
  if (matchesKey(data, "q")) command = "quit";
  else if (matchesKey(data, Key.tab)) command = "next-tab";
  else if (matchesKey(data, Key.shift("tab"))) command = "previous-tab";
  else if (matchesKey(data, "r")) command = "refresh";
  else if (
    tab !== "coordination" &&
    (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace))
  ) command = "coordination";
  return command;
}

function footerHint(tab: DashboardTab, page: Page): string {
  if (tab !== "coordination") {
    return "tab/shift-tab switch · esc coordination · r refresh · q close";
  }
  return page.kind === "detail"
    ? "tab switch · j/k scroll · esc/← back · r refresh · q close"
    : "tab switch · ↑↓ navigate · enter open · esc back · r refresh · q close";
}

interface BodyState {
  list?: SelectList;
  detailScroll?: ScrollView;
}

interface DashboardBodyOptions {
  container: Container;
  page: Page;
  selectedValue?: string;
  snapshot: Snapshot;
  tab: DashboardTab;
  theme: Theme;
  usagePane: UsagePane;
  back: () => void;
  open: (value: string) => void;
}

function createList(
  entries: Entry[],
  selectedValue: string | undefined,
  theme: Theme,
  open: (value: string) => void,
  back: () => void,
): SelectList {
  const list = new SelectList(entries satisfies SelectItem[], Math.min(entries.length, 12), {
    selectedPrefix: (text) => theme.fg("accent", text),
    selectedText: (text) => theme.fg("accent", text),
    description: (text) => theme.fg("muted", text),
    scrollInfo: (text) => theme.fg("dim", text),
    noMatch: (text) => theme.fg("warning", text),
  });
  const selectedIndex = entries.findIndex((entry) => entry.value === selectedValue);
  if (selectedIndex >= 0) list.setSelectedIndex(selectedIndex);
  list.onSelect = (item) => open(item.value);
  list.onCancel = back;
  return list;
}

function addOverviewCounts(container: Container, snapshot: Snapshot, theme: Theme): void {
  const counts = snapshot.dashboard.counts;
  container.addChild(
    new Text(
      `${theme.fg("success", String(counts.active_sessions))} sessions   ${theme.fg("accent", String(counts.active_claims))} claims   ${theme.fg(counts.open_conflicts > 0 ? "error" : "muted", String(counts.open_conflicts))} conflicts`,
      1,
      1,
    ),
  );
}

function addDashboardBody(options: DashboardBodyOptions): BodyState {
  if (options.tab !== "coordination") {
    options.container.addChild(options.usagePane);
    return {};
  }
  if (options.page.kind === "overview") {
    addOverviewCounts(options.container, options.snapshot, options.theme);
  }
  if (options.page.kind === "detail") {
    const detailScroll = new ScrollView(
      new Text(detailText(options.snapshot, options.page), 1, 1),
      {
        primary: true,
        scrollbar: "always",
        scrollbarTrackStyle: (text) => options.theme.fg("dim", text),
        scrollbarThumbStyle: (text) => options.theme.fg("accent", text),
      },
    );
    options.container.addChild(detailScroll);
    return { detailScroll };
  }
  const list = createList(
    entriesFor(options.snapshot, options.page),
    options.selectedValue,
    options.theme,
    options.open,
    options.back,
  );
  options.container.addChild(list);
  return { list };
}

function scrollDetail(data: string, detail: ScrollView | undefined, tui: TUI): boolean {
  let lines = 0;
  if (matchesKey(data, Key.up) || matchesKey(data, "k")) lines = -1;
  if (matchesKey(data, Key.down) || matchesKey(data, "j")) lines = 1;
  if (matchesKey(data, Key.pageUp)) lines = -10;
  if (matchesKey(data, Key.pageDown)) lines = 10;
  if (lines !== 0) {
    detail?.scrollBy(lines);
    tui.requestRender();
  }
  return lines !== 0;
}

interface CoordinationInputOptions {
  page: Page;
  list?: SelectList;
  detail?: ScrollView;
  tui: TUI;
  back: () => void;
}

function handleCoordinationInput(data: string, options: CoordinationInputOptions): void {
  const shouldGoBack = options.page.kind === "detail" && (
    matchesKey(data, Key.escape) ||
    matchesKey(data, Key.backspace) ||
    matchesKey(data, Key.left)
  );
  if (shouldGoBack) options.back();
  else if (options.page.kind !== "detail" || !scrollDetail(data, options.detail, options.tui)) {
    options.list?.handleInput(data);
    options.tui.requestRender();
  }
}

export class NexusDashboardComponent implements Component {
  private container = new Container();
  private list: SelectList | undefined;
  private detailScroll: ScrollView | undefined;
  private page: Page = { kind: "overview" };
  private tab: DashboardTab = "coordination";
  private status = "live";
  private readonly poller: SnapshotPoller;
  private readonly usagePane: UsagePane;

  constructor(
    private readonly tui: TUI,
    private readonly theme: Theme,
    private snapshot: Snapshot,
    private readonly options: DashboardOptions,
  ) {
    this.poller = new SnapshotPoller(
      snapshot,
      options.loadSnapshot,
      (update) => this.applyPollingUpdate(update),
    );
    this.usagePane = new UsagePane(theme, options.loadUsage, () => {
      this.rebuild();
      this.tui.requestRender();
    });
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
    const command = dashboardCommand(data, this.tab);
    switch (command) {
      case "quit": this.options.done(); break;
      case "next-tab": this.switchTab(nextDashboardTab(this.tab, 1)); break;
      case "previous-tab": this.switchTab(nextDashboardTab(this.tab, -1)); break;
      case "refresh": this.refresh(); break;
      case "coordination": this.switchTab("coordination"); break;
      default: {
        if (this.tab === "coordination") {
          handleCoordinationInput(
            data,
            {
              page: this.page,
              list: this.list,
              detail: this.detailScroll,
              tui: this.tui,
              back: () => this.goBack(),
            },
          );
        }
      }
    }
  }

  dispose(): void {
    this.poller.dispose();
    this.usagePane.dispose();
  }

  private applyPollingUpdate(update: PollingUpdate): void {
    this.snapshot = update.snapshot;
    this.status = update.status;
    if (update.mode === "scope" && update.status === "live") {
      this.page = { kind: "overview" };
      this.usagePane.setScope(this.snapshot.dashboard.project_id);
    }
    this.rebuild();
    this.tui.requestRender();
  }

  private switchTab(tab: DashboardTab): void {
    if (tab === this.tab) return;
    const wasAnalytics = this.tab !== "coordination";
    const isAnalytics = tab !== "coordination";
    this.tab = tab;
    if (!wasAnalytics && isAnalytics) {
      this.usagePane.activate(tab as UsageTab, this.snapshot.dashboard.project_id);
    } else if (wasAnalytics && !isAnalytics) {
      this.usagePane.deactivate();
    } else if (isAnalytics) {
      this.usagePane.show(tab as UsageTab);
    }
    this.rebuild();
    this.tui.requestRender();
  }

  private refresh(): void {
    if (this.tab === "coordination") {
      void this.poller.refresh(this.snapshot.dashboard.project_id, "manual");
    } else {
      this.usagePane.refresh();
    }
  }

  private goBack(): void {
    const previous = backPage(this.page);
    if (previous === null) {
      this.options.done();
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
    const body = addDashboardBody({
      container,
      page: this.page,
      selectedValue,
      snapshot: this.snapshot,
      tab: this.tab,
      theme: this.theme,
      usagePane: this.usagePane,
      back: () => this.goBack(),
      open: (value) => this.open(value),
    });
    this.list = body.list;
    this.detailScroll = body.detailScroll;
    this.addFooter(container);
    this.container = container;
  }

  private addHeader(container: Container): void {
    container.addChild(new DynamicBorder((text) => this.theme.fg("accent", text)));
    container.addChild(new Text(renderTabHeader(this.theme, this.tab), 1, 0));
    const title = this.tab === "coordination" ? pageTitle(this.page) : this.tab === "tools" ? "Tool usage" : "Skill usage";
    container.addChild(new Text(this.theme.fg("accent", this.theme.bold(title)), 1, 0));
    const scope = scopeLabel(this.snapshot);
    const status = this.tab === "coordination"
      ? `${scope} · ${this.status} · ${new Date(this.snapshot.dashboard.generated_at).toLocaleTimeString()}`
      : this.usagePane.statusLine(scope);
    container.addChild(
      new Text(
        this.theme.fg("muted", status),
        1,
        0,
      ),
    );
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

  private addFooter(container: Container): void {
    container.addChild(
      new Text(this.theme.fg("dim", footerHint(this.tab, this.page)), 1, 1),
    );
    container.addChild(new DynamicBorder((text) => this.theme.fg("accent", text)));
  }
}
