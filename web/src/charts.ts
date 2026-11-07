// @feature observability-ui
// @spec docs/features/observability-ui.md
import * as d3 from "d3";
import { positionedChart, type Chart, type Choose, type ChartEvent, type ChartInput } from "./chart-model";
import { drawBars } from "./chart-bars";
import { drawTrend } from "./chart-trend";
import { drawWaterfall } from "./chart-waterfall";
import { drawHistogram } from "./chart-histogram";
import { reveal } from "./chart-svg";
export type { Chart, ChartInput, ChartEvent } from "./chart-model";

/** A render replaces presentation; only exact record selections return to Elm. */
export function renderChart(host: HTMLElement, input: ChartInput, selected: (event: ChartEvent) => void): void {
  const chart = positionedChart(input), width = Math.max(300, host.clientWidth || 800);
  const restoreFocus = host.contains(document.activeElement) || host.closest(".waterfall-scroll") === document.activeElement;
  d3.select(host).selectAll("*").remove();
  const description = chart.type === "waterfall" ? "Execution flow. Time flows down; dispatch and observed returns connect activities" : chart.type === "histogram" ? "Runs over time" : "Recorded observations chart";
  const svg = d3.select(host).append("svg").attr("role", "group").attr("aria-label", description);
  const choose = (id: string) => selected({ chart: chart.id, id });
  drawContent(svg, chart, width, choose);
  if (chart.type === "waterfall" && chart.selected) restoreSelection(host, restoreFocus);
}

function restoreSelection(host: HTMLElement, restoreFocus: boolean): void {
  reveal(host);
  if (restoreFocus) host.querySelector<SVGElement>(".waterfall-node.selected")?.focus({ preventScroll: true });
}

function drawContent(svg: Parameters<typeof drawWaterfall>[0], chart: Chart, width: number, choose: Choose): void {
  if (chart.type === "waterfall") drawWaterfall(svg, chart, width, choose);
  else if (chart.type === "bars") drawBars(svg, chart.bars ?? [], width, choose);
  else if (chart.type === "histogram") drawHistogram(svg, chart.bins ?? [], chart.binSeconds ?? 21600, width);
  else drawTrend(svg, chart.points ?? [], width, choose, chart.line ?? true);
}
