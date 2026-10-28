// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
import { chartBars, chartTrend } from "./charts.ts";
import { centeredWindowStart, fitDisplayText } from "./display.ts";
import type { ConcernCounts, ConcernObservation, ConcernSummary, ConcernSessionTitle } from "./concerns.ts";
import type { ConcernsState, ConcernView } from "./concerns-state.ts";
import type { DashboardTheme } from "./theme.ts";
import { concernLocationLines, concernSessionLine } from "./concerns-evidence.ts";

function counts(value: ConcernCounts): string { return `${value.raw_observations} observations · ${value.known_source_occurrences} known sources · ${value.work_sessions} work sessions · ${value.host_sessions} host sessions`; }
function unknowns(value: ConcernCounts): string { return `Unknown source ${value.unknown_source_observations} · work ${value.unknown_work_observations} · host ${value.unknown_host_observations} · occurrence time ${value.unknown_source_time_observations}`; }
function window(state: ConcernsState): string { return `Window: ${state.query.occurred_after ?? "all retained history"}${state.query.occurred_before ? ` → ${state.query.occurred_before}` : ""} · source time, else observation time`; }
function sortMetric(state: ConcernsState, value: ConcernCounts): number {
  return state.query.sort === "returns" ? value.returns_after_resolution : state.query.sort === "recent" ? value.raw_observations : value.work_sessions;
}
function rankedAreas(state: ConcernsState, width: number): string[] {
  const data = state.dashboard!, start = centeredWindowStart(data.areas.length, state.selected, 6);
  return chartBars(data.areas.slice(start, start + 6).map((item, offset) => ({ label: `${start + offset === state.selected ? "› " : ""}${item.area.display_name}`,
    value: sortMetric(state, item.counts), valueLabel: state.query.sort === "returns" ? `${item.counts.returns_after_resolution} returns` : state.query.sort === "recent" ? `${item.counts.raw_observations} observations` : `${item.counts.work_sessions} sessions` })), width);
}
function overview(state: ConcernsState, width: number): string[] {
  const data = state.dashboard; if (!data) return ["Problem areas", state.status];
  const selected = data.areas[state.selected];
  return ["Problem areas · reported problems, not verified health or a quality score", window(state), counts(data.counts), unknowns(data.counts).replace("Unknown", "unknown"),
    "Reported observations over time · UTC · unknown occurrence time uses observation time",
    ...chartTrend(data.daily.map(point => ({ label: point.date, at: Date.parse(point.date), value: point.raw_observations })), width, value => String(value)).slice(0, 5),
    `Ranked code areas · ${state.query.sort ?? "work_sessions"} · m sort · Enter area · a all concerns · n more`, ...rankedAreas(state, width),
    ...(selected ? [`Area: ${selected.area.kind} · ${selected.area.repository ?? "repository unknown"} · ${selected.open_concern_count} open concerns`,
      `Last observed: ${selected.last_observed_at ?? "unknown"} · ${selected.counts.returns_after_resolution} returns · shared areas are not additive`] : ["No code areas reported · a includes unlocated concerns · no reports would not verify health"]),
    `${data.areas.length} areas loaded · ${data.next_cursor ? "more pages · n" : "all returned areas loaded"}${data.partial ? " · PARTIAL source history" : ""}`,
    ...data.notices.slice(0, 2), ...(data.notices.length > 2 ? [`${data.notices.length - 2} additional notices · CLI dashboard for full qualifications`] : [])];
}
function concernPreview(selected: ConcernSummary | undefined): string[] {
  return selected ? [`Selected: ${selected.title}${selected.areas_partial ? " · area mappings partial" : ""}`, selected.description,
    `Hypothesis: ${selected.hypothesis ?? "none reported"}`, `Last: ${selected.last_observed_at ?? "unknown"} · ${selected.lifecycle_qualification ?? "no lifecycle qualification reported"}`] : ["No concerns returned for this area."];
}
function area(view: Extract<ConcernView, { kind: "area" }>): string[] {
  const page = view.page, start = centeredWindowStart(page?.items.length ?? 0, view.selected, 8), selected = page?.items[view.selected];
  return [view.area ? `Code area: ${view.area.area.display_name} · ${view.area.area.kind}` : "All code concerns · includes unlocated reports", view.area ? counts(view.area.counts) : `${page?.total ?? "unknown"} concerns in the selected project`, "Distinct concerns · Enter timeline · n more · Escape areas",
    ...(page?.items.slice(start, start + 8).map((item, offset) => `${start + offset === view.selected ? "›" : " "} ${item.title} · ${item.status} · ${item.counts.work_sessions} work sessions · ${item.counts.returns_after_resolution} returns`) ?? ["Loading concerns…"]),
    ...concernPreview(selected),
    ...(page ? [`${page.items.length}/${page.total} concerns loaded · ${page.next_cursor ? "more · n" : "end of returned population"}${page.partial ? " · PARTIAL" : ""}`] : [])];
}
function preview(value: unknown): string {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length <= 240 ? text : `${text.slice(0, 240)}… [preview omitted; CLI history retains evidence]`;
}
function observationMetadata(item: ConcernObservation, titles?: ConcernSessionTitle[]): string[] {
  const input = item.input, context = input.context;
  const fields = Object.entries(input.body).filter(([key]) => key !== "event");
  return [`Evidence: ${input.id} · ${input.body.event}`, `Occurred: ${input.occurred_at ?? "unknown"} · observed: ${input.observed_at} · received: ${item.recorded_at}`,
    `Reporter: ${item.provenance.agent} · host ${context.host_session_id ?? "unknown"} · work ${context.work_session_id ?? "unknown"}`,
    concernSessionLine(item, titles),
    `Activity: ${context.activity_id ?? "unknown"} · repository: ${context.repository ?? "unknown"}`,
    ...concernLocationLines(item),
    ...fields.slice(0, 6).map(([key, value]) => `${key}: ${preview(value)}`),
    ...(fields.length > 6 ? [`${fields.length - 6} additional evidence fields · CLI get/history for full record`] : []),
    ...(item.source ? ["Imported source receipt retained · CLI get/history for original provenance"] : [])];
}
function detail(view: Extract<ConcernView, { kind: "detail" }>): string[] {
  const value = view.detail, selected = value.history[view.selected], start = centeredWindowStart(value.history.length, view.selected, 6);
  return [`${value.summary.title}${value.summary.areas_partial ? " · area mappings partial" : ""}`, `${value.summary.status} · ${value.summary.id} · ${value.summary.lifecycle_qualification ?? "no lifecycle qualification reported"}`,
    counts(value.summary.counts), "Timeline · observations/remedies/resolutions/returns · ↑/↓ selects evidence · n more",
    ...value.history.slice(start, start + 6).map((item, index) => `${start + index === view.selected ? "›" : " "} ${item.input.occurred_at ?? `${item.input.observed_at} (occurrence unknown)`} ─ ${item.input.body.event}`),
    ...(selected ? observationMetadata(selected, value.session_titles) : ["No retained observations in this detail page."]),
    `${value.history.length} observations loaded · ${value.next_cursor ? "more · n" : "end of returned history"}${value.partial ? " · PARTIAL" : ""}`];
}
export function renderConcerns(state: ConcernsState, width: number, theme: DashboardTheme): { lines: string[]; detailLength: number } {
  const content = Math.max(1, width - 1), view = state.view;
  const lines = view.kind === "overview" ? overview(state, content) : view.kind === "area" ? area(view) : detail(view);
  return { detailLength: 0, lines: lines.map(line => fitDisplayText(` ${line}`, width)).map(line => line.startsWith(" ›") ? theme.fg("accent", line) : line) };
}
