// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
import { truncateToWidth, wrapTextWithAnsi } from "@earendil-works/pi-tui";
import { renderEvalGroup, renderEvalSets } from "./evals-dashboard-view.ts";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import { EVAL_DETAIL_ROWS, type EvalHeader, type EvalLifecycle, type EvalRecord, type EvalValue } from "./evals.ts";
import type { EvalsState, EvalView } from "./evals-state.ts";
import type { DashboardTheme } from "./theme.ts";

interface Preview { lines: string[]; omitted: boolean }
interface Presentation { lines: string[]; detailLength: number }
function preview(text: string): string {
  return text.length <= 800 ? text : `${text.slice(0, 800)}… [display preview truncated; use record/export for full content]`;
}
/** Bounded tree preview keeps opaque provider-shaped values safe without pretending to normalize them. */
function appendValue(output: Preview, key: string, value: EvalValue, depth = 0): void {
  if (output.lines.length >= 240) { output.omitted = true; return; }
  const heading = `${"  ".repeat(Math.min(depth, 8))}${key.replaceAll("_", " ")}`;
  if (value === null || typeof value !== "object") output.lines.push(`${heading}: ${preview(String(value))}`);
  else if (Array.isArray(value)) {
    output.lines.push(`${heading}: ${value.length} items`);
    value.slice(0, 24).forEach((item, index) => appendValue(output, String(index + 1), item, depth + 1));
    if (value.length > 24) output.lines.push(`${heading} [${value.length - 24} items omitted from display]`);
  } else {
    output.lines.push(heading);
    Object.entries(value).forEach(([field, item]) => appendValue(output, field, item, depth + 1));
  }
}
const PRIORITY = ["motivation", "reason", "rationale", "intended_behavior", "expected_behavior", "metric_id", "value", "name", "description", "unit", "value_format", "calculation", "interpretation"];
function fieldPriority(key: string): number { return PRIORITY.indexOf(key) + 1 || 100; }
function lifecycleLines(lifecycle: EvalLifecycle | null | undefined): string[] {
  if (!lifecycle) return [];
  const lines = [`Caller-reported lifecycle · initial ${lifecycle.initial_state}`];
  if (lifecycle.truncated) lines.push("Lifecycle history is partial · current state is not established.");
  if (lifecycle.frontiers.length > 1) lines.push(`Unresolved lifecycle fork · ${lifecycle.frontiers.length} reported frontiers · no winner selected.`);
  if (lifecycle.frontiers.length === 0) lines.push("No later lifecycle reports.");
  for (const event of lifecycle.frontiers) {
    lines.push(`${event.state} · ${event.id} · occurred ${event.occurred_at} · recorded ${event.recorded_at}`);
  }
  return [...lines, ""];
}
export function evalDetailLines(record: EvalRecord): string[] {
  const metric = record.links.find(link => link.relation === "metric_definition");
  const output: Preview = { omitted: false, lines: [record.title, `${record.body.kind} · ${record.id}`, `Project: ${record.project_id}`,
    `Occurred: ${record.occurred_at}`, `Recorded: ${record.recorded_at}`, "", ...lifecycleLines(record.lifecycle),
    ...(metric ? [`Metric definition: ${metric.target_id} · l links for meaning/formula`, ""] : [])] };
  Object.entries(record.body).filter(([key]) => key !== "kind")
    .sort(([left], [right]) => fieldPriority(left) - fieldPriority(right))
    .forEach(([key, value]) => appendValue(output, key, value));
  appendValue(output, "capture provenance", record.provenance);
  if (record.source) appendValue(output, "original source receipt", {
    project_id: record.source.project_id, recorded_at: record.source.recorded_at, provenance: record.source.provenance,
  });
  if (output.omitted) output.lines.push("Display is partial · use record/export for full structured evidence.");
  return output.lines.map(line => sanitizeDisplayText(line, "multiline"));
}
function rowTitle(record: EvalHeader | EvalRecord): string {
  const kind = "body" in record ? record.body.kind : record.kind;
  const date = Date.parse(record.occurred_at);
  const occurred = Number.isFinite(date) ? `${new Date(date).toISOString().slice(0, 16).replace("T", " ")}Z` : record.occurred_at;
  return `${occurred} · ${kind} · ${record.title} · ${record.id}`;
}
function rows(records: (EvalHeader | EvalRecord)[], selected: number, width: number): string[] {
  const start = centeredWindowStart(records.length, selected, 10);
  return records.slice(start, start + 10).map((record, offset) =>
    truncateToWidth(`${start + offset === selected ? "›" : " "} ${rowTitle(record)}`, width));
}
function renderList(state: EvalsState, width: number): string[] {
  const query = state.query;
  const filters = `Filters · ${query.kind ?? "all kinds"} · repo ${query.repository ?? "any"} · model ${query.model ?? "any"} · search ${query.search ?? "none"}`;
  if (!state.page) return [filters, state.status];
  const page = state.page;
  const counts = new Map<string, number>();
  page.records.forEach(record => counts.set(record.kind, (counts.get(record.kind) ?? 0) + 1));
  const lines = [filters, `${page.records.length} records loaded · ${page.next_cursor ? "more records · n next page" : "all matching reported records loaded"}`,
    [...counts].map(([kind, count]) => `${count} ${kind}`).join(" · ")];
  return [...lines, ...rows(page.records, state.selected, width),
    ...(page.records.length === 0 ? ["No matching eval history. Record/import through nexus eval."] : [])];
}
function renderDetail(view: Extract<EvalView, { kind: "detail" }>, width: number): Presentation {
  const lines = [...evalDetailLines(view.record), "", `${view.record.links.length} recorded links · l browse links · w why/history`]
    .flatMap(line => wrapTextWithAnsi(line, width));
  const start = Math.min(view.scroll, Math.max(0, lines.length - EVAL_DETAIL_ROWS));
  return { detailLength: lines.length, lines: [...lines.slice(start, start + EVAL_DETAIL_ROWS),
    ...(start + EVAL_DETAIL_ROWS < lines.length ? ["↓ more below · j/k scroll"] : [])] };
}
function renderHistory(view: Extract<EvalView, { kind: "why" }>, width: number): string[] {
  return [`Why / reported history · ${view.record.id}`,
    view.history.truncated ? "History is partial · use CLI export/query for additional evidence" : "All returned historical links shown · capture gaps remain explicit",
    ...rows(view.history.records, view.selected, width),
    view.history.records.length === 0 ? "No linked rationale reported." : "Enter opens the selected historical record"];
}
function renderLinks(view: Extract<EvalView, { kind: "links" }>, width: number): string[] {
  const links = view.record.links;
  const start = centeredWindowStart(links.length, view.selected, 12);
  return ["Linked evidence and definitions · Enter follows · Escape back", ...links.slice(start, start + 12).map((link, offset) =>
    truncateToWidth(`${start + offset === view.selected ? "›" : " "} ${link.relation} → ${link.target_id}`, width)), ...(links.length === 0 ? ["No linked records reported."] : [])];
}
function renderView(state: EvalsState, width: number): Presentation {
  const view = state.view;
  let presentation: Presentation;
  switch (view.kind) {
    case "dashboard": presentation = { lines: renderEvalSets(state, width), detailLength: 0 }; break;
    case "group": presentation = { lines: renderEvalGroup(state, width), detailLength: 0 }; break;
    case "detail": presentation = renderDetail(view, width); break;
    case "why": presentation = { lines: renderHistory(view, width), detailLength: 0 }; break;
    case "links": presentation = { lines: renderLinks(view, width), detailLength: 0 }; break;
    case "list": presentation = { lines: renderList(state, width), detailLength: 0 }; break;
  }
  return presentation;
}
export function renderEvals(state: EvalsState, width: number, theme: DashboardTheme): Presentation {
  const presented = state.editor ? { lines: [`Filter ${state.editor.field}: ${state.editor.text}▏`, "Enter applies · Escape cancels · empty clears"], detailLength: 0 }
    : renderView(state, Math.max(1, width - 2));
  const lines = presented.lines.flatMap(line => wrapTextWithAnsi(` ${sanitizeDisplayText(line, "multiline")}`, Math.max(1, width)))
    .map(line => line.startsWith(" ›") ? theme.fg("accent", line) : line);
  return { lines, detailLength: presented.detailLength };
}
