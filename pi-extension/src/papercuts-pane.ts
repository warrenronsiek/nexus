// @feature papercuts
// @feature observability-ui
// @spec docs/features/papercuts.md
// @spec docs/features/observability-ui.md
import { Key, matchesKey, truncateToWidth, wrapTextWithAnsi, type Component, type TUI } from "@earendil-works/pi-tui";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import type { Papercut, PapercutsApi, PapercutsList } from "./papercuts.ts";
import type { DashboardTheme } from "./theme.ts";

export class PapercutsPane implements Component {
  private list: PapercutsList | undefined;
  private selected = 0;
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
  private async load(): Promise<void> {
    const request = new AbortController();
    this.pending = request;
    this.status = "loading…";
    this.changed();
    try {
      const result = await this.api.list(this.projectId, request.signal);
      if (this.pending !== request) return;
      this.list = result;
      this.selected = Math.min(this.selected, Math.max(0, result.papercuts.length - 1));
      this.status = "live";
    } catch (error) {
      if (this.pending === request) this.status = `stale · ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      if (this.pending === request) { this.pending = undefined; this.changed(); }
    }
  }
  render(width: number): string[] {
    const cut = this.list?.papercuts[this.selected];
    const contentWidth = Math.max(1, width - 2);
    const lines = this.detail && cut
      ? this.renderDetail(cut, contentWidth)
      : this.renderList(contentWidth);
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
  private renderList(contentWidth: number): string[] {
    if (!this.list) return [` ${sanitizeDisplayText(this.status)}`];
    const lines = [` ${this.list.papercuts.length} open papercuts${this.list.truncated ? " · more not shown (nexus papercuts dump for complete registry)" : ""}`];
    if (this.list.papercuts.length === 0) return [...lines, " No open papercuts."];
    const start = centeredWindowStart(this.list.papercuts.length, this.selected, 12);
    for (const [offset, item] of this.list.papercuts.slice(start, start + 12).entries()) {
      const row = truncateToWidth(sanitizeDisplayText(`${start + offset === this.selected ? "›" : " "} ${item.upvote_count} encounters · ${item.title} · last ${item.last_upvoted_at ?? item.created_at}`), contentWidth);
      lines.push(` ${start + offset === this.selected ? this.theme.fg("accent", row) : row}`);
    }
    return lines;
  }
  handleInput(data: string): void {
    if (matchesKey(data, Key.escape) || matchesKey(data, Key.backspace) || matchesKey(data, Key.left)) this.detail = false;
    else if (matchesKey(data, Key.enter)) { this.detail = true; this.scroll = 0; }
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
    if (size) this.selected = (this.selected + direction + size) % size;
  }
  private changed(): void { this.updated(); this.tui.requestRender(); }
}
