// @feature papercuts
// @feature observability-ui
// @spec docs/features/papercuts.md
// @spec docs/features/observability-ui.md
import { Key, matchesKey, truncateToWidth, wrapTextWithAnsi, type Component, type TUI } from "@earendil-works/pi-tui";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import type { Papercut, PapercutCursor, PapercutsApi, PapercutsList } from "./papercuts.ts";
import type { DashboardTheme } from "./theme.ts";

function acceptPage(previous: PapercutsList | undefined, page: PapercutsList,
  after: PapercutCursor | undefined, selected: number, autoSelectNewPage: boolean): { list: PapercutsList; selected: number } {
  if (after === undefined || previous === undefined) {
    return { list: page, selected: Math.min(selected, Math.max(0, page.papercuts.length - 1)) };
  }
  return {
    list: { papercuts: [...previous.papercuts, ...page.papercuts], truncated: page.truncated },
    selected: page.papercuts.length === 0 || !autoSelectNewPage ? selected : previous.papercuts.length,
  };
}

function readFailure(previous: PapercutsList | undefined, after: PapercutCursor | undefined,
  error: unknown): string {
  const failure = error instanceof Error ? error.message : String(error);
  if (after !== undefined) return `older page unavailable · ${failure} · ↓ retry`;
  if (previous === undefined) return `unavailable · ${failure} · r retry`;
  return `stale · ${failure} · r retry`;
}

function renderPapercutList(list: PapercutsList | undefined, selected: number, contentWidth: number,
  theme: DashboardTheme, status: string): string[] {
  if (!list) return [` ${sanitizeDisplayText(status)}`];
  const lines = [` ${list.papercuts.length} open papercuts loaded${list.truncated ? " · ↓ load older or n next page (nexus papercuts dump for complete history)" : " · all open reports loaded"}`];
  if (list.papercuts.length === 0) return [...lines, " No open papercuts."];
  const start = centeredWindowStart(list.papercuts.length, selected, 12);
  for (const [offset, item] of list.papercuts.slice(start, start + 12).entries()) {
    const row = truncateToWidth(sanitizeDisplayText(`${start + offset === selected ? "›" : " "} ${item.upvote_count} encounters · ${item.title} · last ${item.last_upvoted_at ?? item.created_at}`), contentWidth);
    lines.push(` ${start + offset === selected ? theme.fg("accent", row) : row}`);
  }
  return lines;
}

export class PapercutsPane implements Component {
  private list: PapercutsList | undefined;
  private selected = 0;
  private selectionRevision = 0;
  private detail = false;
  private scroll = 0;
  private detailLines = 0;
  private projectId: string | null = null;
  private active = false;
  private pending: AbortController | undefined;
  private status = "not loaded";
  constructor(private readonly tui: TUI, private readonly theme: DashboardTheme,
    private readonly api: PapercutsApi, private readonly updated: () => void) {}
  activate(projectId: string | null): void {
    this.active = true;
    if (this.projectId !== projectId) { this.list = undefined; this.selected = 0; this.detail = false; }
    this.projectId = projectId;
    this.refresh();
  }
  deactivate(): void { this.active = false; this.pending?.abort(); this.pending = undefined; }
  dispose(): void { this.deactivate(); }
  invalidate(): void {}
  statusLine(): string { return sanitizeDisplayText(`Papercuts · ${this.projectId ?? "current project"} · ${this.status}`); }
  refresh(): void { if (this.active && !this.pending) void this.load(); }
  private loadMore(): void {
    if (!this.active || this.pending || !this.list?.truncated) return;
    const last = this.list.papercuts.at(-1);
    if (last !== undefined) void this.load({ updated_at: last.updated_at, id: last.id });
  }
  private async load(after?: PapercutCursor): Promise<void> {
    const request = new AbortController();
    this.pending = request;
    const previous = this.list;
    const selectionRevision = this.selectionRevision;
    this.status = after === undefined ? "loading…" : "loading older…";
    this.changed();
    try {
      const result = await this.api.list(this.projectId, request.signal, after);
      if (this.pending !== request) return;
      const page = acceptPage(previous, result, after, this.selected,
        this.selectionRevision === selectionRevision && !this.detail);
      this.list = page.list;
      this.selected = page.selected;
      this.status = "live";
    } catch (error) {
      if (this.pending === request) this.status = readFailure(previous, after, error);
    } finally {
      if (this.pending === request) { this.pending = undefined; this.changed(); }
    }
  }
  render(width: number): string[] {
    const cut = this.list?.papercuts[this.selected];
    const contentWidth = Math.max(1, width - 2);
    const lines = this.detail && cut
      ? this.renderDetail(cut, contentWidth)
      : renderPapercutList(this.list, this.selected, contentWidth, this.theme, this.status);
    return lines.flatMap(line => wrapTextWithAnsi(line, Math.max(1, width)));
  }
  private renderDetail(cut: Papercut, contentWidth: number): string[] {
    const lines = [cut.title, `${cut.upvote_count} encounters · ${cut.status}`, `ID: ${cut.id}`,
      `Project: ${cut.project_id}`, `First reported: ${cut.created_at}`,
      `Last encounter: ${cut.last_upvoted_at ?? cut.created_at}`, "", cut.description,
      ...(cut.resolution === null ? [] : [`Resolution: ${cut.resolution}`])]
      .flatMap(line => wrapTextWithAnsi(sanitizeDisplayText(line, "multiline"), contentWidth));
    this.detailLines = lines.length;
    this.scroll = Math.min(this.scroll, Math.max(0, lines.length - 18));
    return lines.slice(this.scroll, this.scroll + 18).map(line => ` ${line}`).concat(
      this.scroll + 18 < lines.length ? [" ↓ more below · j/k scroll"] : []);
  }
  handleInput(data: string): void {
    if (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace) || matchesKey(data, Key.left)) this.detail = false;
    else if (matchesKey(data, Key.enter)) { this.detail = true; this.scroll = 0; this.selectionRevision += 1; }
    else if (!this.detail && matchesKey(data, "n")) this.loadMore();
    else this.move(data);
    this.changed();
  }
  private move(data: string): void {
    const direction = matchesKey(data, Key.up) || matchesKey(data, "k") ? -1 :
      matchesKey(data, Key.down) || matchesKey(data, "j") ? 1 : 0;
    if (this.detail) {
      this.scroll = Math.max(0, Math.min(this.scroll + direction, this.detailLines - 18));
      return;
    }
    const size = this.list?.papercuts.length ?? 0;
    if (size === 0 || direction === 0) return;
    if (direction > 0 && this.selected === size - 1 && this.list?.truncated) {
      this.loadMore();
      return;
    }
    this.selected = (this.selected + direction + size) % size;
    this.selectionRevision += 1;
  }
  private changed(): void { this.updated(); this.tui.requestRender(); }
}
