// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { truncateToWidth, wrapTextWithAnsi } from "@earendil-works/pi-tui";
import { workTimeAxis, workTimeBar, type WorkTimeAxis } from "./work-interval.ts";
import { renderWorkDashboard } from "./work-dashboard.ts";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import type { DashboardTheme } from "./theme.ts";
import type { WorkDetail, WorkLink, WorkObservation, WorkReference, WorkSummary, WorkValue } from "./work.ts";
import { WORK_DETAIL_ROWS, workRows, type WorkState, type WorkTree, type WorkRow } from "./work-state.ts";

/** Decimal placement preserves every reported nanosecond without coercing the exact string to a number. */
export function workDuration(summary: WorkSummary): string {
  if (!summary.elapsed) return summary.finished_at ? "duration unavailable" : "open · duration unknown";
  const text = summary.elapsed.nanoseconds.padStart(10, "0");
  return `${text.slice(0, -9).replace(/^0+(?=\d)/u, "")}.${text.slice(-9)}s (${summary.elapsed.method})`;
}
function flags(summary: WorkSummary): string { return [...summary.conflicts.map(item => `conflict:${item}`), ...summary.gaps.map(item => `gap:${item}`)].join(" · "); }
function summaryLine(summary: WorkSummary): string { return `${summary.title} · ${summary.kind} · ${summary.outcome ?? summary.state} · ${workDuration(summary)}${flags(summary) ? ` · ${flags(summary)}` : ""}`; }
export function workReferenceText(target: WorkReference): string {
  if (target.kind === "revision") return `${target.repository} @ ${target.commit} (${target.object_format})`;
  if (target.kind === "external") return `${target.provider} ${target.type} ${target.id}${target.url ? ` · ${target.url}` : ""}`;
  return `${target.kind}:${target.id}`;
}
function linkLine(link: WorkLink): string { return `${link.relation} → ${workReferenceText(link.target)} · ${link.state}`; }
function preview(value: WorkValue): string {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length <= 1200 ? text : `${text.slice(0, 1200)}… [display preview truncated; use work events/export for full evidence]`;
}
function observationLines(observation: WorkObservation): string[] {
  const lines = [`${observation.body.event} · ${observation.id}`, `  Occurred: ${observation.occurred_at} · received: ${observation.recorded_at}`,
    `  Producer: ${observation.producer.name} ${observation.producer.version} · instance ${observation.producer.instance_id} · ${observation.method}`,
    `  Submitter: ${observation.provenance.agent} · host ${observation.provenance.session_id ?? "unknown"}`];
  if (observation.source) lines.push(`  Original receipt: ${observation.source.project_id} · ${observation.source.recorded_at} · ${observation.source.provenance.agent}`);
  if (observation.correction) lines.push(`  Corrects: ${observation.correction.replaces} · ${observation.correction.reason}`);
  for (const [key, value] of Object.entries(observation.body)) if (key !== "event") lines.push(`  ${key}: ${preview(value)}`);
  return lines;
}
export function workDetailLines(record: WorkDetail): string[] {
  const item = record.summary;
  return [item.title, `${item.subject.scope}:${item.subject.id} · ${item.kind} · ${item.outcome ?? item.state}`,
    `Started: ${item.started_at ?? "unknown"} · finished: ${item.finished_at ?? "not reported"}`,
    `Elapsed: ${workDuration(item)} · inclusive of any nested work`,
    `Conflicts: ${item.conflicts.join(", ") || "none reported"}`, `Capture gaps: ${item.gaps.join(", ") || "none reported; completeness is not guaranteed"}`,
    `Work sessions: ${item.work_session_ids.join(", ") || "unlinked"} · w related session hierarchy · parent: ${item.parent_activity_id ?? "none reported"}`,
    `Repository: ${item.context.repository ?? "unknown"} · host session: ${item.context.host_session_id ?? "unknown"}`,
    `Agent: ${item.context.agent_instance_id ?? "unknown"} · model: ${item.context.model_id ?? "unknown"} · environment: ${item.context.environment ?? "unspecified"}`,
    ...(item.context.revision ? [`Revision: ${item.context.revision.repository} @ ${item.context.revision.commit} (${item.context.revision.object_format})`] : []),
    ...item.context.external.map(external => `External: ${external.provider} ${external.kind}:${external.id} ${external.url ?? ""}`),
    ...Object.entries(item.context.labels).map(([name, value]) => `Label ${name}: ${value}`),
    "", `${record.links.length} recorded links · l browse · e all observations`,
    ...(record.truncated ? ["Detail is partial · page all observations with e"] : []),
    ...record.observations.flatMap(observation => ["", ...observationLines(observation)])];
}
function listLines(state: WorkState, width: number): string[] {
  const query = Object.entries(state.query).filter(([key]) => key !== "after" && key !== "limit").map(([key, value]) => `${key}=${value}`).join(" · ");
  const label = state.listing === "sessions" ? "Work sessions" : "Activity search";
  const lines = [`${label} · f switch sessions/activity search`, `Filters · ${query || "all recorded work"}`];
  if (!state.page) return [...lines, state.status];
  lines.push(`${state.page.items.length} ${state.listing === "sessions" ? "work sessions" : "activities"} loaded · ${state.page.next_cursor ? "more · n next" : "all matching reported records loaded"}`);
  const start = centeredWindowStart(state.page.items.length, state.selected, 10);
  lines.push(...state.page.items.slice(start, start + 10).map((item, offset) => truncateToWidth(`${start + offset === state.selected ? "›" : " "} ${summaryLine(item)}`, width)));
  if (!state.page.items.length) lines.push(`No matching ${state.listing === "sessions" ? "work sessions" : "activities"}. Capture through nexus work or nexus_work.`);
  return lines;
}
function relative(timestamp: string | null, origin: number): string {
  if (timestamp === null || !Number.isFinite(origin) || !Number.isFinite(Date.parse(timestamp))) return "unknown";
  const delta = (Date.parse(timestamp) - origin) / 1000;
  return `${delta >= 0 ? "+" : ""}${delta.toFixed(3)}s`;
}
function timeAxis(tree: WorkTree, width: number): WorkTimeAxis | undefined {
  const boundaries = workRows(tree).flatMap(row => row.kind === "activity" ? [row.summary.started_at, row.summary.finished_at] : []);
  return workTimeAxis([...boundaries, tree.session.started_at], Math.min(20, Math.max(8, Math.floor(width / 6))));
}
interface TreePresentation { tree: WorkTree; width: number; axis?: WorkTimeAxis }
function treeRow(row: WorkRow, selected: boolean, presentation: TreePresentation): string {
  const { tree, axis, width } = presentation;
  const marker = selected ? "›" : " ";
  const indent = "  ".repeat(Math.min(row.depth, 16));
  if (row.kind === "more") return truncateToWidth(`${marker} ${indent}… load more children · Enter/n`, width);
  const item = row.summary;
  const bar = tree.timeline ? `${workTimeBar(item, axis)} ` : "";
  const disclosure = item.has_children ? tree.expanded.has(item.subject.id) ? "▾" : "▸" : "·";
  const origin = tree.session.started_at === null ? NaN : Date.parse(tree.session.started_at);
  const time = tree.timeline ? `[${relative(item.started_at, origin)} → ${item.finished_at === null ? "open/unknown" : relative(item.finished_at, origin)}] ` : "";
  return truncateToWidth(`${marker} ${bar}${indent}${disclosure} ${time}${summaryLine(item)}`, width);
}
function treeLines(tree: WorkTree, width: number): string[] {
  const rows = workRows(tree);
  const origin = tree.session.started_at === null ? NaN : Date.parse(tree.session.started_at);
  const axis = tree.timeline ? timeAxis(tree, width) : undefined;
  const lines = [`${tree.session.title} · ${tree.session.subject.id}`, `${tree.timeline ? "Relative timeline" : "Activity hierarchy"} · ${rows.filter(row => row.kind === "activity").length} visible activities · →/space expand · Enter detail · t switch`,
    `Origin: ${tree.session.started_at ?? "unknown"} · intervals are inclusive; parallel siblings overlap`, `Session capture gaps: ${tree.session.gaps.join(", ") || "none reported"}`];
  if (tree.timeline) lines.push(axis ? `Time axis: ${relative(new Date(axis.start).toISOString(), origin)} → ${relative(new Date(axis.end).toISOString(), origin)} · = observed interval · ? finish unknown` : "Time axis unavailable: source boundaries missing");
  const start = centeredWindowStart(rows.length, tree.selected, 12);
  lines.push(...rows.slice(start, start + 12).map((row, offset) => treeRow(row, start + offset === tree.selected, { tree, axis, width })));
  return [...lines, ...treeNotices(tree, rows.length)];
}
function treeNotices(tree: WorkTree, count: number): string[] {
  const lines: string[] = [];
  const pages = [...tree.pages.values()];
  const unresolved = pages.flatMap(page => page.unresolved_links);
  if (pages.some(page => page.truncated)) lines.push("Timeline is partial · n pages children · s session detail for links");
  if (unresolved.length) lines.push(`${unresolved.length} unresolved/shared link reports · s session detail then l links`, ...unresolved.slice(0, 3).map(linkLine));
  if (count === 0) lines.push("No recorded root activities. Child capture may be missing; s shows session observations/links.");
  if (count >= 2048) lines.push("Display traversal bounded at 2048 rows · collapse branches or query the CLI.");
  return lines;
}
function scrollLines(lines: string[], scroll: number, width: number): { lines: string[]; detailLength: number } {
  const wrapped = lines.flatMap(line => wrapTextWithAnsi(sanitizeDisplayText(line, "multiline"), width));
  const start = Math.min(scroll, Math.max(0, wrapped.length - WORK_DETAIL_ROWS));
  return { lines: [...wrapped.slice(start, start + WORK_DETAIL_ROWS), ...(start + WORK_DETAIL_ROWS < wrapped.length ? ["↓ more below · j/k scroll"] : [])], detailLength: wrapped.length };
}
function renderLinks(view: Extract<WorkState["view"], { kind: "links" }>): string[] {
  const start = centeredWindowStart(view.record.links.length, view.selected, 12);
  return ["Recorded links · Enter follows activity/session · external/eval/revision references are shown as evidence", ...view.record.links.slice(start, start + 12).map((link, offset) =>
    `${start + offset === view.selected ? "›" : " "} ${linkLine(link)}`), ...(view.record.links.length ? [] : ["No links reported."])];
}
function membershipLines(view: Extract<WorkState["view"], { kind: "memberships" }>): string[] {
  const ids = view.record.summary.work_session_ids;
  const start = centeredWindowStart(ids.length, view.selected, 12);
  return ["Related work sessions · Enter opens the canonical hierarchy", ...ids.slice(start, start + 12).map((id, index) => `${start + index === view.selected ? "›" : " "} session:${id}`),
    ...(ids.length ? [] : ["No work-session association recorded."])];
}
function selectableLines(state: WorkState, view: Exclude<WorkState["view"], { kind: "detail" | "events" }>, width: number): string[] {
  switch (view.kind) {
    case "dashboard": return renderWorkDashboard(view, width, state.status);
    case "list": return listLines(state, width);
    case "tree": return treeLines(view.tree, width);
    case "memberships": return membershipLines(view);
    case "links": return renderLinks(view);
  }
}
function renderView(state: WorkState, width: number): { lines: string[]; detailLength: number } {
  const view = state.view;
  if (view.kind === "detail") return scrollLines(workDetailLines(view.record), view.scroll, width);
  if (view.kind === "events") return scrollLines(["Immutable observations · occurrence and receipt times are separate", `${view.page.observations.length} loaded · ${view.page.next_cursor ? "n more" : "all returned observations loaded"}`,
    ...view.page.observations.flatMap(item => ["", ...observationLines(item)])], view.scroll, width);
  return { lines: selectableLines(state, view, width), detailLength: 0 };
}
export function renderWork(state: WorkState, width: number, theme: DashboardTheme): { lines: string[]; detailLength: number } {
  const result = state.editor ? { detailLength: 0, lines: [`Filter ${state.editor.field}: ${state.editor.text}▏`, "Enter applies · Escape cancels · empty clears"] }
    : renderView(state, Math.max(1, width - 2));
  return { detailLength: result.detailLength, lines: result.lines.flatMap(line => wrapTextWithAnsi(` ${sanitizeDisplayText(line, "multiline")}`, Math.max(1, width))).map(line => line.startsWith(" ›") ? theme.fg("accent", line) : line) };
}
