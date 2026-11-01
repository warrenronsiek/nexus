// @feature observability-ui
// @spec docs/features/observability-ui.md
import * as d3 from "d3";
import { positionedChart, type ChartEvent, type ChartInput } from "./chart-model";
import { drawBars } from "./chart-bars";
import { drawTrend } from "./chart-trend";
import { drawWaterfall } from "./chart-waterfall";
import { reveal } from "./chart-svg";
export { layoutWaterfall } from "./waterfall-layout";
export type { Chart, ChartInput, ChartEvent } from "./chart-model";

/** A render replaces presentation; only exact record selections return to Elm. */
export function renderChart(host: HTMLElement, input: ChartInput, selected: (event: ChartEvent) => void): void {
  const chart = positionedChart(input), width = Math.max(300, host.clientWidth || 800);
  d3.select(host).selectAll("*").remove();
  const description = chart.type === "waterfall" ? "Session task waterfall; time flows downward" : "Recorded observations chart";
  const svg = d3.select(host).append("svg").attr("role", "img").attr("aria-label", description);
  const choose = (id: string) => selected({ chart: chart.id, id });
  if (chart.type === "waterfall") drawWaterfall(svg, chart, width, choose);
  else if (chart.type === "bars") drawBars(svg, chart.bars ?? [], width, choose);
  else drawTrend(svg, chart.points ?? [], width, choose);
  if (chart.type === "waterfall" && chart.selected) reveal(host);
}
