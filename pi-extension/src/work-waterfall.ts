// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { truncateToWidth } from "@earendil-works/pi-tui";
import { centeredWindowStart, fitDisplayText } from "./display.ts";
import { formatDurationSeconds } from "./charts.ts";
import { activityStyle } from "./work-activity-style.ts";
import type { DashboardTheme } from "./theme.ts";
import type { WorkSubject, WorkSummary } from "./work.ts";

export interface WaterfallNode extends Pick<WorkSummary, "title" | "kind" | "parent_activity_id" | "started_at" | "finished_at" | "state" | "outcome" | "elapsed" | "conflicts" | "gaps"> { id: string; executor: string | null; accounting_role?: "container" | "execution" | "wait" | "unknown"; queued_at?: string | null; queue_elapsed?: WorkSummary["elapsed"] }
interface PlacedNode { node: WaterfallNode; lane: number; parent: number | null; queue: number | null; start: number | null; end: number | null; unresolved: boolean }
export interface WaterfallLayout { origin: number; end: number; nodes: PlacedNode[]; lanes: number }
export interface WaterfallView { offset: number; zoom: number; laneOffset: number; width?: number; rows?: number }
interface WaterfallPresentation { selected?: WorkSubject; theme?: DashboardTheme; rows?: number }
export type WaterfallMove = "up" | "down" | "page_up" | "page_down" | "home" | "end" | "left" | "right" | "zoom_in" | "zoom_out";
const TIME_ROWS = 12;
function time(value: string | null): number | null { const result = value === null ? NaN : Date.parse(value); return Number.isFinite(result) ? result : null; }
export function initialWaterfall(): WaterfallView { return { offset: 0, zoom: 0, laneOffset: 0 }; }
/** Allocate lanes from canonical parentage; overlapping siblings cannot share a lane. */
export function layoutWaterfall(root: WaterfallNode, activities: WaterfallNode[]): WaterfallLayout {
  const all = [root, ...activities];
  const times = all.flatMap(node => [time(node.queued_at ?? null), time(node.started_at), time(node.finished_at)]).filter((value): value is number => value !== null);
  const origin = Math.min(...times), end = Math.max(...times);
  return new LaneLayout(all, origin, end).build();
}
class LaneLayout {
  private readonly placed = new Map<WaterfallNode, PlacedNode>();
  private readonly occupied: PlacedNode[][] = [];
  private readonly indexed: Map<string, WaterfallNode>;
  constructor(private readonly all: WaterfallNode[], private readonly origin: number, private readonly end: number) { this.indexed = new Map(all.slice(1).map(node => [node.id, node])); }
  build(): WaterfallLayout {
    for (const node of this.all) for (const ancestor of this.lineage(node).reverse()) this.place(ancestor);
    return { origin: this.origin, end: this.end, nodes: [...this.placed.values()], lanes: this.occupied.length };
  }
  private lineage(node: WaterfallNode): WaterfallNode[] {
    const chain: WaterfallNode[] = [], seen = new Set<WaterfallNode>();
    let cursor: WaterfallNode | undefined = node;
    while (cursor && !this.placed.has(cursor) && !seen.has(cursor)) {
      chain.push(cursor); seen.add(cursor);
      cursor = this.parentNode(cursor);
    }
    return chain;
  }
  private parentNode(node: WaterfallNode): WaterfallNode | undefined {
    if (node === this.all[0]) return;
    return node.parent_activity_id === null ? this.all[0] : this.indexed.get(node.parent_activity_id);
  }
  private freeLane(minimum: number, start: number | null, end: number | null): number {
    let lane = minimum;
    while (this.occupied[lane]?.some(item => (start ?? this.origin) <= (item.end ?? this.end) && (end ?? this.end) >= (item.queue ?? item.start ?? this.origin))) lane++;
    return lane;
  }
  private place(node: WaterfallNode): PlacedNode {
    const existing = this.placed.get(node); if (existing) return existing;
    const isRoot = node === this.all[0], parentNode = this.parentNode(node);
    const parent = parentNode ? this.placed.get(parentNode) : undefined, start = time(node.started_at), end = time(node.finished_at);
    const queue = time(node.queued_at ?? null), lane = this.freeLane(isRoot ? 0 : (parent?.lane ?? 0) + 1, queue ?? start, end);
    const item = { node, lane, parent: parent?.lane ?? null, queue, start, end, unresolved: !isRoot && !parent };
    (this.occupied[lane] ??= []).push(item); this.placed.set(node, item); return item;
  }
}
function scale(layout: WaterfallLayout, view: WaterfallView): { step: number; rows: number } {
  const span = Math.max(0, layout.end - layout.origin), step = Math.max(1, span / (((view.rows ?? TIME_ROWS) - 1) * 2 ** view.zoom));
  return { step, rows: Math.ceil(span / step) + 1 };
}
export function moveWaterfall(view: WaterfallView, layout: WaterfallLayout, direction: WaterfallMove): void {
  const height = view.rows ?? TIME_ROWS;
  const deltas: Partial<Record<WaterfallMove, number>> = { up: -1, down: 1, page_up: 1 - height, page_down: height - 1 };
  view.offset += deltas[direction] ?? 0;
  switch (direction) {
    case "home": view.offset = 0; break;
    case "end": view.offset = scale(layout, view).rows; break;
    case "left": view.laneOffset--; break;
    case "right": view.laneOffset++; break;
    case "zoom_in": zoomWaterfall(view, layout, 1); break;
    case "zoom_out": zoomWaterfall(view, layout, -1); break;
  }
  view.offset = Math.max(0, Math.min(view.offset, Math.max(0, scale(layout, view).rows - height)));
  view.laneOffset = Math.max(0, Math.min(view.laneOffset, Math.max(0, layout.lanes - 1)));
}
function selectedNode(layout: WaterfallLayout, subject?: WorkSubject): PlacedNode | undefined {
  if (!subject) return;
  return subject.scope === "session" ? layout.nodes[0] : layout.nodes.slice(1).find(item => item.node.id === subject.id);
}
function graphColumns(width: number): number { return Math.max(1, Math.min(61, Math.floor(width / 2) - 12)); }
/** Reveal source admission/start and its lane together, retaining exact scoped identity. */
export function revealWaterfall(view: WaterfallView, layout: WaterfallLayout, subject: WorkSubject): void {
  const item = selectedNode(layout, subject); if (!item) return;
  const height = view.rows ?? TIME_ROWS, lanes = Math.floor((graphColumns(view.width ?? 80) - 1) / 4) + 1;
  if (item.lane < view.laneOffset) view.laneOffset = item.lane;
  if (item.lane >= view.laneOffset + lanes) view.laneOffset = item.lane - lanes + 1;
  const boundary = item.queue ?? item.start ?? item.end;
  if (boundary === null || !Number.isFinite(layout.origin)) return;
  const target = Math.round((boundary - layout.origin) / scale(layout, view).step);
  if (target < view.offset || target >= view.offset + height) view.offset = Math.max(0, target - Math.floor(height / 2));
  view.offset = Math.min(view.offset, Math.max(0, scale(layout, view).rows - height));
}
function zoomWaterfall(view: WaterfallView, layout: WaterfallLayout, delta: number): void {
  const timestamp = view.offset * scale(layout, view).step;
  view.zoom = Math.max(0, Math.min(24, view.zoom + delta));
  view.offset = Math.round(timestamp / scale(layout, view).step);
}
const JUNCTIONS = [" ", "╵", "╷", "│", "╶", "└", "┌", "├", "╴", "┘", "┐", "┤", "─", "┴", "┬", "┼"];
interface Canvas { cells: number[][]; marks: Map<string, string>; labels: Map<number, { text: string; node: WaterfallNode }[]>; owners: Map<string, WaterfallNode>; selected?: WaterfallNode; theme?: DashboardTheme; offset: number; step: number; origin: number; laneOffset: number; columns: number; rows: number }
function column(canvas: Canvas, lane: number): number { return (lane - canvas.laneOffset) * 4; }
function row(canvas: Canvas, timestamp: number): number { return Math.round((timestamp - canvas.origin) / canvas.step) - canvas.offset; }
function ink(canvas: Canvas, y: number, x: number, bits: number, node: WaterfallNode): void {
  if (y < 0 || y >= canvas.rows || x < 0 || x >= canvas.columns) return;
  canvas.cells[y]![x]! |= bits;
  if (!canvas.selected || canvas.owners.get(`${y}:${x}`) !== canvas.selected) canvas.owners.set(`${y}:${x}`, node);
}
function branch(canvas: Canvas, item: PlacedNode, y: number): void {
  if (item.parent === null) return;
  const from = column(canvas, item.parent), to = column(canvas, item.lane);
  for (let x = Math.max(0, from); x <= Math.min(canvas.columns - 1, to); x++) ink(canvas, y, x, (x > from ? 8 : 0) | (x < to ? 4 : 0), item.node);
}
function draw(canvas: Canvas, item: PlacedNode, horizon: number): void {
  const x = column(canvas, item.lane), first = item.start === null ? null : row(canvas, item.start), last = item.end === null ? null : row(canvas, item.end);
  if (first !== null) {
    drawSpan(canvas, { x, first, last, horizon: row(canvas, horizon) }, item.node);
    if (item.queue === null) branch(canvas, item, first);
  }
  drawQueue(canvas, item, horizon);
  drawBoundary(canvas, item, { y: first, x, marker: "●", boundary: "start" });
  drawBoundary(canvas, item, { y: last, x, marker: "■", boundary: "end" });
}
function drawQueue(canvas: Canvas, item: PlacedNode, horizon: number): void {
  if (item.queue === null) return;
  const x = column(canvas, item.lane), first = row(canvas, item.queue), last = row(canvas, item.start ?? horizon);
  for (let y = Math.max(0, first); y < Math.min(canvas.rows, last); y++) {
    ink(canvas, y, x, 3, item.node);
    if (x >= 0 && x < canvas.columns) canvas.marks.set(`${y}:${x}`, "┆");
  }
  branch(canvas, item, first);
  drawBoundary(canvas, item, { y: first, x, marker: "◌", boundary: "queued" });
}
function drawSpan(canvas: Canvas, span: { x: number; first: number; last: number | null; horizon: number }, node: WaterfallNode): void {
  for (let y = Math.max(0, span.first); y <= Math.min(canvas.rows - 1, span.last ?? span.horizon); y++) {
    ink(canvas, y, span.x, (y > span.first ? 1 : 0) | (span.last === null || y < span.last ? 2 : 0), node);
    if (span.last === null && y > span.first && span.x >= 0 && span.x < canvas.columns) canvas.marks.set(`${y}:${span.x}`, "┊");
  }
}
function boundaryFlags(item: PlacedNode): string {
  const flags = [item.start === null ? "start unknown" : "", item.end === null ? "finish unknown" : "", item.unresolved ? "unresolved parent" : "", item.node.conflicts.length ? "conflict" : ""].filter(Boolean);
  return flags.length ? ` (${flags.join(", ")})` : "";
}
function drawBoundary(canvas: Canvas, item: PlacedNode, point: { y: number | null; x: number; marker: string; boundary: string }): void {
  const { y, x } = point;
  if (y === null || y < 0 || y >= canvas.rows || x < 0 || x >= canvas.columns) return;
  canvas.marks.set(`${y}:${x}`, item.node.conflicts.length ? "?" : point.marker);
  canvas.owners.set(`${y}:${x}`, item.node);
  const labels = canvas.labels.get(y) ?? [];
  labels.push({ text: `${item.node === canvas.selected ? "› " : ""}${item.node.title} ${point.boundary}${boundaryFlags(item)}`, node: item.node }); canvas.labels.set(y, labels);
}
function styled(canvas: Pick<Canvas, "theme" | "selected">, node: WaterfallNode | undefined, text: string): string {
  if (!node || !canvas.theme) return text;
  const colored = canvas.theme.fg(activityStyle(node.kind).color, text);
  return node === canvas.selected ? canvas.theme.bold(colored) : colored;
}
function canvasLine(canvas: Canvas, y: number, end: number): string {
  const delta = Math.min(end - canvas.origin, (canvas.offset + y) * canvas.step) / 1000;
  const graph = canvas.cells[y]!.map((bits, x) => {
    const node = canvas.owners.get(`${y}:${x}`), glyph = canvas.marks.get(`${y}:${x}`) ?? JUNCTIONS[bits]!;
    return styled(canvas, node, node === canvas.selected && glyph === "│" ? "┃" : glyph);
  }).join("");
  const labels = (canvas.labels.get(y) ?? []).sort((a, b) => Number(b.node === canvas.selected) - Number(a.node === canvas.selected));
  const label = `+${Number(delta.toFixed(Math.min(3, Math.max(0, Math.ceil(-Math.log10(canvas.step / 1000))))))}s`;
  return `${label.padStart(10)} ${graph} ${labels.slice(0, 3).map(item => styled(canvas, item.node, fitDisplayText(item.text, canvas.columns * 4))).join(" · ")}${labels.length > 3 ? ` · +${labels.length - 3} boundaries` : ""}`;
}
function newCanvas(layout: WaterfallLayout, view: WaterfallView, width: number, options: WaterfallPresentation): Canvas {
  const height = view.rows ?? TIME_ROWS, { step, rows } = scale(layout, view), offset = Math.min(view.offset, Math.max(0, rows - height));
  return { cells: Array.from({ length: height }, () => Array<number>(graphColumns(width)).fill(0)), marks: new Map(), labels: new Map(), owners: new Map(), selected: selectedNode(layout, options.selected)?.node, theme: options.theme, offset, step, origin: layout.origin, laneOffset: view.laneOffset, columns: graphColumns(width), rows: height };
}
function unknownWaterfall(layout: WaterfallLayout, view: WaterfallView, width: number, subject?: WorkSubject): string[] {
  const selected = selectedNode(layout, subject), height = view.rows ?? TIME_ROWS;
  const start = centeredWindowStart(layout.nodes.length, selected ? layout.nodes.indexOf(selected) : 0, height);
  return ["Time axis unavailable · no source boundaries", ...layout.nodes.slice(start, start + height).map(item =>
    `${item === selected ? "› " : ""}${item.node.title} · start unknown · finish unknown`)].map(line => fitDisplayText(line, width));
}
function kindLegend(theme?: DashboardTheme): string {
  return `Kind: ${["agent", "test", "script", "review", "tool", "other"].map(kind => {
    const style = activityStyle(kind); return theme?.fg(style.color, style.label) ?? style.label;
  }).join(" · ")}`;
}
function graphLines(layout: WaterfallLayout, view: WaterfallView, canvas: Canvas): string[] {
  const { step, offset, columns } = canvas;
  const lines = [`Time ↓ · ${formatDurationSeconds(step / 1000)}/row · lanes → ${view.laneOffset + 1}–${Math.min(layout.lanes, view.laneOffset + Math.floor(columns / 4) + 1)}/${layout.lanes}`,
    "↑/↓ j/k task · ← parent → child · PgUp/PgDn time · H/L lanes · +/- zoom"];
  for (let y = 0; y < canvas.rows && offset + y < scale(layout, view).rows; y++) lines.push(canvasLine(canvas, y, layout.end));
  return lines;
}
export function renderWaterfall(layout: WaterfallLayout, view: WaterfallView, width: number, options: WaterfallPresentation = {}): string[] {
  view.width = width;
  if (options.rows !== undefined) view.rows = Math.max(6, Math.min(24, options.rows));
  if (!Number.isFinite(layout.origin)) return unknownWaterfall(layout, view, width, options.selected);
  const canvas = newCanvas(layout, view, width, options);
  for (const item of layout.nodes) draw(canvas, item, layout.end);
  const lines = graphLines(layout, view, canvas);
  lines.push("● start · ■ end · ◌ queued · ┆ queue wait · ┊ finish unknown · ? conflict · UTC positions");
  lines.push(kindLegend(options.theme));
  const unknown = layout.nodes.filter(item => item.start === null && item.end === null).length;
  if (unknown) lines.push(`${unknown} tasks without source boundaries · j/k selects their exact evidence`);
  return lines.map(line => line.includes("\u001b") ? truncateToWidth(line, Math.max(1, width), "…") : fitDisplayText(line, width));
}
