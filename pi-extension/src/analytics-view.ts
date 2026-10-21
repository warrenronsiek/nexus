// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { stripVTControlCharacters } from "node:util";
import { wrapTextWithAnsi, truncateToWidth } from "@earendil-works/pi-tui";
import { centeredWindowStart, sanitizeDisplayText } from "./display.ts";
import type { AnalyticsBundle, Catalog, Examples, ExampleFilter } from "./analytics-data.ts";
export type View = "activities" | "capabilities" | "opportunities" | "legacy";
export interface Entry { label: string; detail: string[]; filter: Omit<ExampleFilter, "offset"> }
export function entries(data: AnalyticsBundle, catalog: Catalog, view: View): Entry[] {
  if (view === "capabilities") return catalog.capabilities.map((item) => ({
    label: `${item.name} · ${item.calls} calls · ${item.offered_sessions} offered sessions`,
    detail: [item.name, `${item.calls} calls · ${item.offered_sessions} offered sessions`, item.description, `${item.kind} · ${item.origin}`, `${item.discoverable_sessions} discoverable sessions · availability ${item.availability}`, "Zero observed uses require inventory coverage; unobserved availability is unknown."],
    filter: { resource_key: item.resource_key },
  }));
  if (view === "opportunities") return data.opportunities.opportunities.map((item) => ({
    label: `${item.name} · ${item.calls} ad-hoc calls · ${item.sessions} sessions`,
    detail: [item.name, `Offered alternatives: ${item.supplied_alternatives.join(", ") || "none observed; inventories may be incomplete"}`, item.evidence], filter: { category_id: item.category_id },
  }));
  return data.summary.activities.map((item) => ({
    label: `${item.name} · ${item.calls} calls · ${item.actions} operations · ${item.sessions} sessions`,
    detail: [item.name, `${item.improvised_actions} operations through inline code, generated scripts or standard CLIs`, "Model descriptions infer purpose; they do not prove each code branch executed."], filter: { category_id: item.category_id },
  }));
}
function exampleLines(page: Examples | undefined): string[] {
  if (!page) return [];
  if (page.examples.length === 0) return ["No observed implementations in this window."];
  return page.examples.flatMap((item) => ["", item.name, item.description, `${item.origin} · ${item.evidence} · ${item.model_id ?? "no model"}`, `${item.tool_name} · ${item.call_outcome} · ${item.observed_at}`]);
}

interface Presentation {
  data: AnalyticsBundle; catalog: Catalog; view: View; selected: number; detail?: Entry; examples?: Examples; scroll: number; page: number; status: string;
}
export function renderAnalytics(state: Presentation, width: number): { lines: string[]; maximumScroll: number } {
  const header = ["1 Activities · 2 Capabilities · 3 Opportunities · 4 Legacy", `${state.data.summary.observed_calls} calls · ${state.data.summary.identified_actions} operations · ${state.data.summary.unclassified_actions} unclassified`];
  const lines = state.detail ? [...state.detail.detail, state.data.summary.counting, ...exampleLines(state.examples)] : listLines(state).map((line) => stripVTControlCharacters(truncateToWidth(sanitizeDisplayText(line), Math.max(1, width), "…")));
  const wrap = (line: string): string[] => wrapTextWithAnsi(sanitizeDisplayText(line), Math.max(1, width));
  const wrapped = lines.flatMap(wrap);
  const maximumScroll = Math.max(0, wrapped.length - 14);
  const scroll = Math.min(state.scroll, maximumScroll);
  const body = state.detail ? wrapped.slice(scroll, scroll + 14) : wrapped;
  const footer = [state.status, "Calls can overlap. Coverage begins with capture."];
  return { lines: [...header.flatMap(wrap), ...body, ...footer.flatMap(wrap)], maximumScroll };
}
function listLines(state: Presentation): string[] {
  const rows = entries(state.data, state.catalog, state.view);
  const start = centeredWindowStart(rows.length, state.selected, 12);
  const lines = rows.slice(start, start + 12).map((row, index) => `${index + start === state.selected ? "›" : " "} ${row.label}`);
  if (rows.length === 0) lines.push("No activity observed in this window.");
  if (state.data.summary.truncated || state.data.opportunities.truncated) lines.push("Category summary truncated; inspect capabilities and their implementations.");
  if (state.view === "capabilities") lines.push(`Page ${state.page} · n/p pages`);
  return lines;
}
