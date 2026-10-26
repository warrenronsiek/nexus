// @feature work-history
// @spec docs/features/work-history.md
// @feature evals
// @spec docs/features/evals.md
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
import { EvalsPane } from "./evals-pane.ts";
import type { EvalsApi } from "./evals.ts";
import { WorkPane } from "./work-pane.ts";
import type { WorkApi } from "./work.ts";
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
  evalsApi?: EvalsApi;
  workApi?: WorkApi;
  done: () => void;
}

interface DashboardPanes {
  usage: UsagePane;
  memory: MemoryPane;
  papercuts: PapercutsPane;
  sos: SosPane;
  evals: EvalsPane;
  work: WorkPane;
}

type DashboardPaneKind = "coordination" | "usage" | "memory" | "sos" | "papercuts" | "evals" | "work";

function paneKind(tab: DashboardTab): DashboardPaneKind {
  if (tab === "tools" || tab === "skills") return "usage";
  return tab;
}

function deactivatePane(kind: DashboardPaneKind, panes: DashboardPanes): void {
  if (kind === "usage") panes.usage.deactivate();
  else if (kind === "memory") panes.memory.deactivate();
  else if (kind === "sos") panes.sos.deactivate();
  else if (kind === "papercuts") panes.papercuts.deactivate();
  else if (kind === "evals") panes.evals.deactivate();
  else if (kind === "work") panes.work.deactivate();
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
  else if (kind === "evals") panes.evals.activate(projectId);
  else if (kind === "work") panes.work.activate(projectId);
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
  evals: "Eval results",
  work: "Work durations",
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
  if (options.tab === "coordination") return `${options.scope} · ${options.coordinationStatus} · ${new Date(options.generatedAt).toLocaleTimeString()}`;
  if (options.tab === "tools" || options.tab === "skills") return options.panes.usage.statusLine(options.scope);
  return options.panes[options.tab].statusLine();
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

const TAB_HINTS: Record<Exclude<DashboardTab, "coordination">, string> = {
  work: "tab switch · ↑↓ session · enter duration breakdown · [/] activity trend · d records · esc back · r refresh · q close",
  evals: "tab switch · ↑↓ eval set/metric · enter results · [/] metric · b records · d evidence · o repo · m model · / search · esc back · r refresh · q close",
  sos: "tab switch · ↑↓ navigate · enter detail · j/k scroll · esc back · r refresh · q close",
  papercuts: "tab switch · ↑↓ navigate · n load older · enter detail · j/k scroll · esc back · r refresh · q close",
  memory: "tab switch · ↑↓ navigate · enter expand/detail · / search · a add · f invalidate summary · c retry consolidation · r refresh · q close",
  tools: "1 activities · 2 capabilities · 3 opportunities · 4 legacy · ↑↓ select · enter detail · n/p pages · esc back · r refresh · q close",
  skills: "tab/shift-tab switch · esc coordination · r refresh · q close",
};
function footerHint(tab: DashboardTab, page: Page): string {
  if (tab !== "coordination") return TAB_HINTS[tab];
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
  evalsPane: EvalsPane;
  workPane: WorkPane;
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
      `${theme.fg("success", String(counts.active_sessions))} recent sessions   ${theme.fg("accent", String(counts.active_claims))} live claims   ${theme.fg(counts.open_conflicts > 0 ? "error" : "muted", String(counts.open_conflicts))} live conflicts`,
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
      evals: options.evalsPane,
      work: options.workPane,
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
  private readonly evalsPane: EvalsPane;
  private readonly workPane: WorkPane;

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
    const unavailable = async (): Promise<never> => { throw new Error("Eval history API is unavailable in this host"); };
    this.evalsPane = new EvalsPane(tui, theme, options.evalsApi ?? { dashboard: unavailable, list: unavailable, get: unavailable, why: unavailable }, () => this.rebuild());
    const workUnavailable = async (): Promise<never> => { throw new Error("Work history API is unavailable in this host"); };
    this.workPane = new WorkPane(tui, theme, options.workApi ?? { dashboard: workUnavailable, list: workUnavailable, activities: workUnavailable, get: workUnavailable, timeline: workUnavailable, events: workUnavailable }, () => this.rebuild());
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
    const priority: Partial<Record<DashboardTab, { handleInput(data: string): boolean }>> = {
      tools: this.usagePane, evals: this.evalsPane, work: this.workPane,
    };
    if (priority[this.tab]?.handleInput(data)) return;
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
    this.evalsPane.dispose();
    this.workPane.dispose();
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
      evals: this.evalsPane,
      work: this.workPane,
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
      else if (this.tab === "evals") this.evalsPane.refresh();
      else if (this.tab === "work") this.workPane.refresh();
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
      evalsPane: this.evalsPane,
      workPane: this.workPane,
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
        evals: this.evalsPane,
      work: this.workPane,
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
