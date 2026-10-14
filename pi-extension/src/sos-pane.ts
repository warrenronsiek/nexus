// @feature sos
// @feature observability-ui
// @spec docs/features/sos.md
// @spec docs/features/observability-ui.md
import {
  Key,
  matchesKey,
  truncateToWidth,
  wrapTextWithAnsi,
  type Component,
  type TUI,
} from "@earendil-works/pi-tui";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import type { SosApi, SosIssue, SosIssueDetail, SosList } from "./sos.ts";
import type { DashboardTheme as Theme } from "./theme.ts";

type SosMode = "list" | "detail";

interface SosPaneState {
  active: boolean;
  generation: number;
  action: number;
  activeAction: number | undefined;
  mode: SosMode;
  list: SosList | undefined;
  detail: SosIssueDetail | undefined;
  selectedIndex: number;
  detailScroll: number;
  detailLineCount: number;
  status: string;
}

const MAX_VISIBLE_ISSUES = 12;
const MAX_VISIBLE_DETAIL_LINES = 18;

function initialState(): SosPaneState {
  return {
    active: false,
    generation: 0,
    action: 0,
    activeAction: undefined,
    mode: "list",
    list: undefined,
    detail: undefined,
    selectedIndex: 0,
    detailScroll: 0,
    detailLineCount: 0,
    status: "not loaded",
  };
}

function issueCountLabel(count: number): string {
  return `${count} ${count === 1 ? "report" : "reports"}`;
}

function localDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}

function selectedIssue(state: SosPaneState): SosIssue | undefined {
  return state.list?.issues[state.selectedIndex];
}

function renderIssueRow(
  theme: Theme,
  width: number,
  issue: SosIssue,
  selected: boolean,
): string[] {
  const title = sanitizeDisplayText(issue.title);
  const summary = sanitizeDisplayText(issue.summary);
  const lastArrival = sanitizeDisplayText(localDate(issue.last_reported_at));
  const line = truncateToWidth(
    `${selected ? "›" : " "} ${issueCountLabel(issue.importance_count)} · last ${lastArrival} · ${title}`,
    Math.max(1, width - 2),
  );
  return [
    ` ${selected ? theme.fg("accent", line) : line}`,
    `   ${theme.fg("muted", truncateToWidth(summary, Math.max(1, width - 4)))}`,
  ];
}

function renderIssueList(theme: Theme, width: number, state: SosPaneState): string[] {
  if (state.list === undefined) return [` ${theme.fg("muted", sanitizeDisplayText(state.status))}`];
  const triage = state.list.untriaged_report_count;
  const lines = [
    ` ${theme.fg(triage > 0 ? "warning" : "muted", `${triage} awaiting triage`)} · ${state.list.issues.length} aggregate issues${state.list.truncated ? " · more not shown" : ""}`,
    ` ${theme.fg("dim", sanitizeDisplayText(state.status))}`,
    "",
  ];
  if (state.list.issues.length === 0) {
    lines.push(` ${theme.fg("muted", "No current SOS issues.")}`);
    return lines;
  }
  const start = centeredWindowStart(
    state.list.issues.length,
    state.selectedIndex,
    MAX_VISIBLE_ISSUES,
  );
  const end = Math.min(state.list.issues.length, start + MAX_VISIBLE_ISSUES);
  for (let index = start; index < end; index += 1) {
    const issue = state.list.issues[index];
    if (issue === undefined) continue;
    lines.push(...renderIssueRow(theme, width, issue, index === state.selectedIndex));
  }
  return lines;
}

function detailLines(detail: SosIssueDetail): string[] {
  const issue = detail.issue;
  const lines = [
    `${issue.title}`,
    `${issue.importance_count} reports · ${issue.status} · first ${localDate(issue.first_reported_at)} · last ${localDate(issue.last_reported_at)}`,
    issue.summary,
    ...(issue.resolution === null ? [] : [`Resolution: ${issue.resolution}`]),
    "",
    "Source reports",
  ];
  for (const report of detail.reports) {
    lines.push(
      `${report.created_at} · ${report.environment} · version ${report.version} · ${report.model} · ${report.type}`,
      report.message,
      `trace: ${report.trace || "(empty)"}`,
      `metadata: ${JSON.stringify(report.metadata)}`,
      `reporter: ${report.reporter.agent} · ${report.reporter.session_id}`,
      "",
    );
  }
  if (detail.reports_truncated) lines.push("more source reports not shown", "");
  lines.push("Triage attempts");
  if (detail.triage_attempts.length === 0) lines.push("awaiting triage");
  for (const attempt of detail.triage_attempts) {
    lines.push(
      `${attempt.created_at} · ${attempt.outcome} · ${attempt.provider} · ${attempt.model_id ?? "default model"}`,
      attempt.rationale,
    );
  }
  if (detail.triage_attempts_truncated) lines.push("more triage attempts not shown");
  lines.push("", "Lifecycle");
  if (detail.lifecycle_events.length === 0) lines.push("no lifecycle changes");
  for (const event of detail.lifecycle_events) {
    lines.push(`${event.created_at} · ${event.transition} · ${event.reason}`);
  }
  if (detail.lifecycle_events_truncated) lines.push("more lifecycle events not shown");
  return lines;
}

function renderDetail(theme: Theme, width: number, state: SosPaneState): string[] {
  if (state.detail === undefined) return [` ${theme.fg("muted", sanitizeDisplayText(state.status))}`];
  const contentWidth = Math.max(1, width - 2);
  const wrapped = detailLines(state.detail).flatMap((line) => (
    wrapTextWithAnsi(sanitizeDisplayText(line, "multiline"), contentWidth)
  ));
  state.detailLineCount = wrapped.length;
  const maximum = Math.max(0, wrapped.length - MAX_VISIBLE_DETAIL_LINES);
  const start = Math.min(state.detailScroll, maximum);
  const end = Math.min(wrapped.length, start + MAX_VISIBLE_DETAIL_LINES);
  const lines = [` ${theme.fg("dim", sanitizeDisplayText(state.status))}`, ""];
  if (start > 0) lines.push(` ${theme.fg("dim", `↑ ${start} lines above`)}`);
  lines.push(...wrapped.slice(start, end).map((line) => ` ${line}`));
  if (end < wrapped.length) lines.push(` ${theme.fg("dim", `↓ ${wrapped.length - end} lines below`)}`);
  return lines;
}

export class SosPane implements Component {
  private readonly state = initialState();

  constructor(
    private readonly tui: TUI,
    private readonly theme: Theme,
    private readonly api: SosApi,
    private readonly updated: () => void,
  ) {}

  render(width: number): string[] {
    return this.state.mode === "list"
      ? renderIssueList(this.theme, width, this.state)
      : renderDetail(this.theme, width, this.state);
  }

  invalidate(): void {}

  activate(): void {
    if (this.state.active) return;
    this.state.active = true;
    this.state.generation += 1;
    void this.loadList();
  }

  deactivate(): void {
    this.state.active = false;
    this.state.generation += 1;
    this.state.activeAction = undefined;
  }

  refresh(): void {
    if (this.state.active) void this.loadList();
  }

  dispose(): void {
    this.deactivate();
  }

  statusLine(): string {
    return sanitizeDisplayText(`SOS · ${this.state.status}`);
  }

  handleInput(data: string): void {
    if (this.state.mode === "list") this.handleListInput(data);
    else this.handleDetailInput(data);
  }

  private handleListInput(data: string): void {
    if (matchesKey(data, Key.up) || matchesKey(data, "k")) this.move(-1);
    else if (matchesKey(data, Key.down) || matchesKey(data, "j")) this.move(1);
    else if (matchesKey(data, Key.enter)) void this.openSelected();
  }

  private handleDetailInput(data: string): void {
    if (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace) || matchesKey(data, Key.left)) {
      this.state.mode = "list";
      this.state.detail = undefined;
      this.state.detailScroll = 0;
      this.changed();
    } else if (matchesKey(data, Key.up) || matchesKey(data, "k")) {
      this.scroll(-1);
    } else if (matchesKey(data, Key.down) || matchesKey(data, "j")) {
      this.scroll(1);
    } else if (matchesKey(data, Key.pageUp)) {
      this.scroll(-10);
    } else if (matchesKey(data, Key.pageDown)) {
      this.scroll(10);
    }
  }

  private move(direction: -1 | 1): void {
    const size = this.state.list?.issues.length ?? 0;
    if (size === 0) return;
    this.state.selectedIndex = (this.state.selectedIndex + direction + size) % size;
    this.changed();
  }

  private scroll(lines: number): void {
    const maximum = Math.max(0, this.state.detailLineCount - MAX_VISIBLE_DETAIL_LINES);
    this.state.detailScroll = Math.max(0, Math.min(this.state.detailScroll + lines, maximum));
    this.changed();
  }

  private begin(status: string): { generation: number; action: number } | undefined {
    if (!this.state.active || this.state.activeAction !== undefined) return;
    const running = { generation: this.state.generation, action: ++this.state.action };
    this.state.activeAction = running.action;
    this.state.status = status;
    this.changed();
    return running;
  }

  private current(running: { generation: number; action: number }): boolean {
    return this.state.active &&
      this.state.generation === running.generation &&
      this.state.activeAction === running.action;
  }

  private finish(running: { generation: number; action: number }): void {
    if (this.current(running)) this.state.activeAction = undefined;
  }

  private async loadList(): Promise<void> {
    const running = this.begin("loading…");
    if (running === undefined) return;
    try {
      const list = await this.api.list();
      if (!this.current(running)) return;
      this.state.list = list;
      this.state.mode = "list";
      this.state.detail = undefined;
      this.state.selectedIndex = Math.min(
        this.state.selectedIndex,
        Math.max(0, list.issues.length - 1),
      );
      this.state.status = "live";
    } catch (error) {
      if (this.current(running)) this.state.status = `stale · ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      this.finish(running);
      this.changed();
    }
  }

  private async openSelected(): Promise<void> {
    const issue = selectedIssue(this.state);
    if (issue === undefined) return;
    const running = this.begin("loading detail…");
    if (running === undefined) return;
    try {
      const detail = await this.api.get(issue.id);
      if (!this.current(running)) return;
      this.state.detail = detail;
      this.state.mode = "detail";
      this.state.detailScroll = 0;
      this.state.status = "live";
    } catch (error) {
      if (this.current(running)) this.state.status = `stale · ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      this.finish(running);
      this.changed();
    }
  }

  private changed(): void {
    this.updated();
    this.tui.requestRender();
  }
}
