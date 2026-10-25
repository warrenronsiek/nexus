// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
// @boundary calculation-preview: optional JSON calculation metadata supplies a readable formula; retained source remains exact and is never executed.
import { chartBars, chartTrend } from "./charts.ts";
import { centeredWindowStart, fitDisplayText, sanitizeDisplayText } from "./display.ts";
import type { EvalDashboard, EvalMeasurement, EvalSeries, EvalSet } from "./evals-dashboard.ts";
import { visibleEvalSets, type EvalsState } from "./evals-state.ts";

const fit = fitDisplayText;
function date(text: string): string {
  const value = Date.parse(text);
  return Number.isFinite(value) ? new Date(value).toISOString().slice(0, 16).replace("T", " ") + "Z" : text;
}
function coverage(dashboard: EvalDashboard): string[] {
  return [...(dashboard.truncated ? ["Partial history · dashboard read limit reached; totals cover retained observations only."] : []),
    ...dashboard.notices.slice(0, 2), ...(dashboard.notices.length > 2 ? [`${dashboard.notices.length - 2} additional coverage notices · b records for evidence`] : [])];
}
function composition(group: EvalSet, agents = 3): string[] {
  return [`Repository: ${group.repository.join(" + ") || "unknown"}`, `Models: ${group.models.join(" + ") || "unknown"}`,
    `Target: ${group.target_identity}`, ...group.components.slice(0, agents).map(component => `Agent: ${component}`),
    ...(group.components.length > agents ? [`${group.components.length - agents} additional agents · target revisions retained in records`] : [])];
}
export function renderEvalSets(state: EvalsState, width: number): string[] {
  const filters = `Filters · repo ${state.query.repository ?? "any"} · model ${state.query.model ?? "any"} · search ${state.query.search ?? "none"}`;
  if (!state.dashboard) return ["Evaluation sets", filters, state.status];
  const dashboard = state.dashboard;
  const groups = visibleEvalSets(state);
  const selected = groups[state.groupSelected];
  const start = centeredWindowStart(groups.length, state.groupSelected, 6);
  const rows = groups.slice(start, start + 6).map((group, offset) => ({
    label: `${start + offset === state.groupSelected ? "›" : " "} ${group.label}`, value: group.run_count, valueLabel: `${group.run_count} runs / ${group.result_count} results`,
  }));
  return ["Evaluation sets · runs by suite, target, repository and model composition",
    `All captured history · ${dashboard.run_count} runs · ${dashboard.result_count} results · ${groups.length} matching sets`, filters,
    ...chartBars(rows, width), ...(groups.length > 6 ? [`Showing ${start + 1}–${Math.min(start + 6, groups.length)} of ${groups.length} sets · ↑↓ select`] : []),
    ...(selected ? [`Selected: ${selected.label}`, ...composition(selected),
      `Reported states: ${Object.entries(selected.statuses).map(([status, count]) => `${count} ${status}`).join(" · ") || "unknown"}`] :
      ["No matching evaluation sets. Definitions and caller results remain available with b records."]),
    ...coverage(dashboard), "Enter results over time · ↑↓ set · o repository · m model · / search · b records"].map(line => fit(line, width));
}
function exactValue(value: EvalMeasurement): string {
  switch (value.storage) {
    case "inline": return value.text === "" ? '"" (empty text)' : value.text;
    case "artifact": return `artifact ${value.artifact_id}`;
    case "unavailable": return `unavailable · ${value.reason}`;
  }
}
function valuePreview(text: string, width: number): string {
  const value = sanitizeDisplayText(text);
  if (value.length > Math.max(20, width - 16)) return `${value.slice(0, Math.max(1, width - 32))}… [preview; d full]`;
  return value;
}
function valueFrequencies(series: EvalSeries, width: number): string[] {
  const counts = new Map<string, { label: string; count: number }>();
  for (const point of series.points) {
    const key = JSON.stringify(point.value);
    const previous = counts.get(key);
    counts.set(key, { label: exactValue(point.value), count: (previous?.count ?? 0) + 1 });
  }
  const rows = [...counts.values()].sort((left, right) => right.count - left.count);
  return ["No numeric chart · Exact-value frequencies", ...chartBars(rows.slice(0, 4).map(row => ({ label: row.label, value: row.count, valueLabel: String(row.count) })), width),
    ...(rows.length > 4 ? [`${rows.length - 4} additional distinct exact values · d latest result · b records`] : [])];
}
function calculationPreview(text: string): string {
  try {
    const value: unknown = JSON.parse(text);
    if (value === null || typeof value !== "object" || Array.isArray(value)) return text;
    const fields = value as Record<string, unknown>;
    for (const key of ["formula", "description", "instructions"]) {
      if (typeof fields[key] === "string" && fields[key]) return fields[key];
    }
  } catch { /* Plain caller descriptions remain valid previews. */ }
  return text;
}
function compactCoverage(dashboard: EvalDashboard): string[] {
  return [...(dashboard.truncated ? ["Partial history · source/display limits; totals qualified."] : []),
    ...(dashboard.notices.length ? [`${dashboard.notices.length} coverage notices · ${dashboard.notices[0]}`] : [])];
}
function correctionSummary(series: EvalSeries): string[] {
  const correction = series.points.at(-1)?.replaces ? "Latest is a correction" : "";
  const replaced = series.superseded_count ? `${series.superseded_count} superseded results retained` : "";
  const summary = [correction, replaced].filter(Boolean).join(" · ");
  return summary ? [summary] : [];
}
function metricLines(series: EvalSeries, width: number, position: string): string[] {
  const numeric = series.points.some(point => point.numeric_value !== null);
  const latest = series.points.at(-1);
  return [`${position} ${series.label} · ${series.points.length} current results`,
    `Unit: ${series.unit} · scope: ${series.subject_scope} · definition: ${series.definition_id}`, series.description, `Calculation: ${calculationPreview(series.calculation)}`,
    ...(numeric ? chartTrend(series.points.map(point => ({ label: date(point.occurred_at), at: Date.parse(point.occurred_at), value: point.numeric_value })), width,
      value => String(Number(value.toPrecision(5)))) : valueFrequencies(series, width)),
    ...(latest ? [`Latest ${date(latest.occurred_at)}: ${valuePreview(exactValue(latest.value), width)}`] : ["No results captured for this metric definition."]),
    ...correctionSummary(series)];
}
export function renderEvalGroup(state: EvalsState, width: number): string[] {
  const dashboard = state.dashboard;
  if (!dashboard) return ["Evaluation results", state.status];
  const group = dashboard.groups.find(group => group.id === dashboard.selected_group_id);
  const metric = dashboard.series[state.metricSelected];
  return [`${group?.label ?? "Evaluation set"} · results over time`, ...(group ? composition(group, 2) : []),
    `${group?.run_count ?? 0} runs · ${group?.result_count ?? 0} results · ${group?.suite_revision_ids.length ?? 0} suite revisions`,
    ...(metric ? metricLines(metric, width, `[${state.metricSelected + 1}/${dashboard.series.length}]`) :
      ["No measurements or grades recorded for this eval set yet."]),
    ...compactCoverage(dashboard), "Esc sets · [ ] metric · d evidence · b records"].map(line => fit(line, width));
}
