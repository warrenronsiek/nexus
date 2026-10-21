// @feature papercuts
// @spec docs/features/papercuts.md
// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @feature sos
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/sos.md
import { DashboardBorder, selectListTheme, type DashboardTheme as Theme } from "./theme.ts";
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
import { sanitizeDisplayText } from "./display.ts";
import { MemoryPane, type MemoryInteractionPrompts } from "./memory-pane.ts";
import type { MemoryApi } from "./memory.ts";
import { PapercutsPane } from "./papercuts-pane.ts";
import type { PapercutsApi } from "./papercuts.ts";
import { SosPane } from "./sos-pane.ts";
import type { SosApi } from "./sos.ts";
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
  type AnalyticsOptions,
} from "./usage-pane.ts";

export { nextDashboardTab } from "./usage-pane.ts";

interface DashboardOptions extends AnalyticsOptions {
  loadSnapshot: SnapshotLoader;
  loadUsage: UsageLoader;
  papercutsApi: PapercutsApi;
  sosApi: SosApi;
  memoryApi: MemoryApi;
  memoryPrompts: MemoryInteractionPrompts;
  done: () => void;
}

interface DashboardPanes {
  usage: UsagePane;
  memory: MemoryPane;
  papercuts: PapercutsPane;
  sos: SosPane;
}

type DashboardPaneKind = "coordination" | "usage" | "memory" | "sos" | "papercuts";

function paneKind(tab: DashboardTab): DashboardPaneKind {
  if (tab === "tools" || tab === "skills") return "usage";
  return tab;
}

function deactivatePane(kind: DashboardPaneKind, panes: DashboardPanes): void {
  if (kind === "usage") panes.usage.deactivate();
  else if (kind === "memory") panes.memory.deactivate();
  else if (kind === "sos") panes.sos.deactivate();
  else if (kind === "papercuts") panes.papercuts.deactivate();
}

function activatePane(
  tab: DashboardTab,
  projectId: string | null,
  panes: DashboardPanes,
): void {
  const kind = paneKind(tab);
  if (kind === "usage") panes.usage.activate(tab as UsageTab, projectId);
  else if (kind === "memory") panes.memory.activate();
  else if (kind === "sos") panes.sos.activate();
  else if (kind === "papercuts") panes.papercuts.activate(projectId);
}

function transitionPane(
  previous: DashboardTab,
  next: DashboardTab,
  projectId: string | null,
  panes: DashboardPanes,
): void {
  const previousKind = paneKind(previous);
  const nextKind = paneKind(next);
  if (previousKind === nextKind) {
    if (nextKind === "usage") panes.usage.show(next as UsageTab);
    return;
  }
  deactivatePane(previousKind, panes);
  activatePane(next, projectId, panes);
}

const TAB_TITLES: Record<Exclude<DashboardTab, "coordination">, string> = {
  papercuts: "Papercuts",
  sos: "SOS issues",
  tools: "Tool usage",
  skills: "Skill usage",
  memory: "Memory",
};

function dashboardTitle(tab: DashboardTab, page: Page): string {
  return tab === "coordination" ? pageTitle(page) : TAB_TITLES[tab];
}

interface DashboardStatusOptions {
  tab: DashboardTab;
  scope: string;
  coordinationStatus: string;
  generatedAt: string;
  panes: DashboardPanes;
}

function dashboardStatus(options: DashboardStatusOptions): string {
  switch (options.tab) {
    case "coordination":
      return `${options.scope} · ${options.coordinationStatus} · ${new Date(options.generatedAt).toLocaleTimeString()}`;
    case "tools":
    case "skills":
      return options.panes.usage.statusLine(options.scope);
    case "papercuts":
      return options.panes.papercuts.statusLine();
    case "sos":
      return options.panes.sos.statusLine();
    case "memory":
      return options.panes.memory.statusLine();
  }
}

type DashboardCommand = "quit" | "next-tab" | "previous-tab" | "refresh" | "coordination";

function dashboardCommand(data: string, tab: DashboardTab): DashboardCommand | undefined {
  let command: DashboardCommand | undefined;
  if (matchesKey(data, "q")) command = "quit";
  else if (matchesKey(data, Key.tab)) command = "next-tab";
  else if (matchesKey(data, Key.shift("tab"))) command = "previous-tab";
  else if (matchesKey(data, "r")) command = "refresh";
  else if (
    (tab === "tools" || tab === "skills") &&
    (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace))
  ) command = "coordination";
  return command;
}

function footerHint(tab: DashboardTab, page: Page): string {
  if (tab === "sos" || tab === "papercuts") {
    return "tab switch · ↑↓ navigate · enter detail · j/k scroll · esc back · r refresh · q close";
  }
  if (tab === "memory") {
    return "tab switch · ↑↓ navigate · enter expand/detail · / search · a add · f invalidate summary · c retry consolidation · r refresh · q close";
  }
  if (tab === "tools") return "1 activities · 2 capabilities · 3 opportunities · 4 legacy · ↑↓ select · enter detail · n/p pages · esc back · r refresh · q close";
  if (tab === "skills") {
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
  memoryPane: MemoryPane;
  papercutsPane: PapercutsPane;
  sosPane: SosPane;
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
  const list = new SelectList(entries satisfies SelectItem[], Math.min(entries.length, 12), selectListTheme(theme));
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
    const panes: Record<Exclude<DashboardTab, "coordination">, Component> = {
      tools: options.usagePane,
      skills: options.usagePane,
      memory: options.memoryPane,
      sos: options.sosPane,
      papercuts: options.papercutsPane,
    };
    options.container.addChild(panes[options.tab]);
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
  private readonly memoryPane: MemoryPane;
  private readonly papercutsPane: PapercutsPane;
  private readonly sosPane: SosPane;

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
    }, options);
    this.memoryPane = new MemoryPane(
      tui,
      theme,
      options.memoryApi,
      options.memoryPrompts,
      () => this.rebuild(),
    );
    this.papercutsPane = new PapercutsPane(tui, theme, options.papercutsApi, () => this.rebuild());
    this.sosPane = new SosPane(tui, theme, options.sosApi, () => this.rebuild());
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
    if (this.tab === "tools" && this.usagePane.handleInput(data)) return;
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
        } else if (this.tab === "memory") {
          this.memoryPane.handleInput(data);
        } else if (this.tab === "papercuts") {
          this.papercutsPane.handleInput(data);
        } else if (this.tab === "sos") {
          this.sosPane.handleInput(data);
        }
      }
    }
  }

  dispose(): void {
    this.poller.dispose();
    this.usagePane.dispose();
    this.memoryPane.dispose();
    this.sosPane.dispose();
    this.papercutsPane.dispose();
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
    const previous = this.tab;
    this.tab = tab;
    transitionPane(previous, tab, this.snapshot.dashboard.project_id, {
      usage: this.usagePane,
      memory: this.memoryPane,
      papercuts: this.papercutsPane,
      sos: this.sosPane,
    });
    this.rebuild();
    this.tui.requestRender();
  }

  private refresh(): void {
    if (this.tab === "coordination") {
      void this.poller.refresh(this.snapshot.dashboard.project_id, "manual");
    } else if (this.tab === "tools" || this.tab === "skills") {
      this.usagePane.refresh();
    } else {
      if (this.tab === "memory") this.memoryPane.refresh();
      else if (this.tab === "papercuts") this.papercutsPane.refresh();
      else this.sosPane.refresh();
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
      memoryPane: this.memoryPane,
      papercutsPane: this.papercutsPane,
      sosPane: this.sosPane,
      back: () => this.goBack(),
      open: (value) => this.open(value),
    });
    this.list = body.list;
    this.detailScroll = body.detailScroll;
    this.addFooter(container);
    this.container = container;
  }

  private addHeader(container: Container): void {
    container.addChild(new DashboardBorder((text) => this.theme.fg("accent", text)));
    container.addChild(new Text(renderTabHeader(this.theme, this.tab), 1, 0));
    const title = dashboardTitle(this.tab, this.page);
    container.addChild(new Text(this.theme.fg("accent", this.theme.bold(title)), 1, 0));
    const scope = scopeLabel(this.snapshot);
    const status = dashboardStatus({
      tab: this.tab,
      scope,
      coordinationStatus: this.status,
      generatedAt: this.snapshot.dashboard.generated_at,
      panes: {
        usage: this.usagePane,
        memory: this.memoryPane,
        papercuts: this.papercutsPane,
        sos: this.sosPane,
      },
    });
    container.addChild(
      new Text(
        this.theme.fg("muted", sanitizeDisplayText(status)),
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
    container.addChild(new DashboardBorder((text) => this.theme.fg("accent", text)));
  }
}
