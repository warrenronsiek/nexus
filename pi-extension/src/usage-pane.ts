// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
import type { DashboardTheme as Theme } from "./theme.ts";
import type { Component } from "@earendil-works/pi-tui";
import type { UsageItem, UsageSummary } from "./domain.ts";
import { UsagePoller, type UsageLoader } from "./polling.ts";
import { renderUsageBars } from "./usage.ts";

export type DashboardTab = "coordination" | "tools" | "skills" | "memory";
export type UsageTab = Extract<DashboardTab, "tools" | "skills">;

const DASHBOARD_TABS: readonly DashboardTab[] = [
  "coordination",
  "tools",
  "skills",
  "memory",
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
    const label = tab[0].toUpperCase() + tab.slice(1);
    const text = `[ ${label} ]`;
    return tab === active
      ? theme.fg("accent", theme.bold(text))
      : theme.fg("dim", text);
  }).join(" ");
}

function captureHealth(usage: UsageSummary): string {
  return Object.entries(usage.capture_health)
    .filter((entry): entry is [string, number] => typeof entry[1] === "number")
    .map(([name, count]) => `${name.replaceAll("_", " ")} ${count}`)
    .join(" · ");
}

export class UsagePane implements Component {
  private tab: UsageTab = "tools";
  private status = "not loaded";
  private usage: UsageSummary | undefined;
  private readonly poller: UsagePoller;

  constructor(
    private readonly theme: Theme,
    load: UsageLoader,
    private readonly updated: () => void,
  ) {
    this.poller = new UsagePoller(load, (update) => {
      this.usage = update.usage;
      this.status = update.status;
      this.updated();
    });
  }

  render(width: number): string[] {
    if (this.usage === undefined) {
      return [` ${this.theme.fg("muted", "Waiting for observed usage…")}`];
    }
    const items: readonly UsageItem[] = this.tab === "tools"
      ? this.usage.tools
      : this.usage.skills;
    const lines = renderUsageBars(items, Math.max(1, width - 2)).map((line) => ` ${line}`);
    const health = captureHealth(this.usage);
    return health.length === 0
      ? lines
      : [...lines, "", ` ${this.theme.fg("dim", `Capture health · ${health}`)}`];
  }

  invalidate(): void {}

  activate(tab: UsageTab, projectId: string | null): void {
    this.tab = tab;
    this.poller.activate(projectId);
  }

  show(tab: UsageTab): void {
    this.tab = tab;
  }

  deactivate(): void {
    this.poller.deactivate();
  }

  setScope(projectId: string | null): void {
    this.poller.setScope(projectId);
  }

  refresh(): void {
    this.poller.manualRefresh();
  }

  dispose(): void {
    this.poller.dispose();
  }

  statusLine(scope: string): string {
    const through = this.usage === undefined
      ? "seven-day window"
      : `through ${new Date(this.usage.window_ended_at).toLocaleString()}`;
    return `${scope} · ${this.status} · last 7 days · observed by Nexus · ${through}`;
  }
}
