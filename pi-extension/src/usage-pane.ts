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
import type { DashboardTheme as Theme } from "./theme.ts";
import type { Component } from "@earendil-works/pi-tui";
import type { UsageItem, UsageSummary } from "./domain.ts";
import { sanitizeDisplayText } from "./display.ts";
import { UsagePoller, type UsageLoader } from "./polling.ts";
import { AnalyticsBrowser, type AnalyticsApi } from "./analytics.ts";
import { renderUsageBars } from "./usage.ts";

export type DashboardTab = "coordination" | "sos" | "tools" | "skills" | "memory" | "papercuts";
export type UsageTab = Extract<DashboardTab, "tools" | "skills">;

const DASHBOARD_TABS: readonly DashboardTab[] = [
  "coordination",
  "sos",
  "tools",
  "skills",
  "memory",
  "papercuts",
];

export function nextDashboardTab(
  current: DashboardTab,
  direction: 1 | -1,
): DashboardTab {
  const currentIndex = DASHBOARD_TABS.indexOf(current);
  return DASHBOARD_TABS[
    (currentIndex + direction + DASHBOARD_TABS.length) % DASHBOARD_TABS.length
  ];
}

export function renderTabHeader(theme: Theme, active: DashboardTab): string {
  return DASHBOARD_TABS.map((tab) => {
    const label = tab === "sos" ? "SOS" : tab[0].toUpperCase() + tab.slice(1);
    const text = `[ ${label} ]`;
    return tab === active
      ? theme.fg("accent", theme.bold(text))
      : theme.fg("dim", text);
  }).join(" ");
}

function captureHealth(usage: UsageSummary): string {
  return sanitizeDisplayText(Object.entries(usage.capture_health)
    .filter((entry): entry is [string, number] => typeof entry[1] === "number")
    .map(([name, count]) => `${name.replaceAll("_", " ")} ${count}`)
    .join(" · "));
}

export interface AnalyticsOptions { loadAnalyticsUsage?: UsageLoader; analyticsApi?: AnalyticsApi }

export class UsagePane implements Component {
  private browser?: AnalyticsBrowser;
  private project: string | null = null;
  private tab: UsageTab = "tools";
  private status = "not loaded";
  private usage: UsageSummary | undefined;
  private readonly poller: UsagePoller;

  constructor(
    private readonly theme: Theme,
    load: UsageLoader,
    private readonly updated: () => void,
    options: AnalyticsOptions = {},
  ) {
    if (options.analyticsApi) this.browser = new AnalyticsBrowser(options.analyticsApi, updated);
    this.poller = new UsagePoller((project, signal) => this.tab === "tools" && options.loadAnalyticsUsage ? options.loadAnalyticsUsage(project, signal) : load(project, signal), (update) => {
      this.usage = update.usage;
      this.status = update.status;
      if (update.usage?.analytics) this.browser?.update(update.usage.analytics, this.project);
      this.updated();
    });
  }

  render(width: number): string[] {
    if (this.usage === undefined) {
      return [` ${this.theme.fg("muted", "Waiting for observed usage…")}`];
    }
    if (this.tab === "tools" && this.usage.analytics && !this.browser?.legacy && this.browser) return this.browser.render(width);
    const warning = this.tab === "tools" && this.usage.analytics_error ? [this.usage.analytics_error, ""] : [];
    const items: readonly UsageItem[] = this.tab === "tools"
      ? this.usage.tools
      : this.usage.skills;
    const lines = renderUsageBars(items, Math.max(1, width - 2)).map((line) => ` ${line}`);
    const health = captureHealth(this.usage);
    return health.length === 0
      ? [...warning, ...lines]
      : [...warning, ...lines, "", ` ${this.theme.fg("dim", `Capture health · ${health}`)}`];
  }

  handleInput(data: string): boolean { return this.tab === "tools" && this.usage?.analytics !== undefined && this.browser?.handleInput(data) === true; }

  invalidate(): void {}

  activate(tab: UsageTab, projectId: string | null): void {
    this.tab = tab;
    if (projectId !== this.project) { this.browser?.clear(); this.usage = undefined; }
    this.project = projectId;
    this.poller.activate(projectId);
  }

  show(tab: UsageTab): void {
    this.browser?.deactivate();
    this.tab = tab;
    this.poller.activate(this.project);
  }

  deactivate(): void {
    this.browser?.deactivate();
    this.poller.deactivate();
  }

  setScope(projectId: string | null): void {
    if (projectId !== this.project) { this.browser?.clear(); this.usage = undefined; }
    this.project = projectId;
    this.poller.setScope(projectId);
  }

  refresh(): void {
    this.browser?.refresh();
    this.poller.manualRefresh();
  }

  dispose(): void {
    this.browser?.deactivate();
    this.poller.dispose();
  }

  statusLine(scope: string): string {
    const through = this.usage === undefined
      ? "seven-day window"
      : `through ${new Date(this.usage.window_ended_at).toLocaleString()}`;
    return sanitizeDisplayText(
      `${scope} · ${this.status} · last 7 days · observed by Nexus · ${through}`,
    );
  }
}
