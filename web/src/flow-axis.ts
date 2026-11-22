// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import { element, type Group, type Svg } from "./chart-svg";
import type { FlowTime } from "./flow-time";

function drawFlowAxis(svg: Svg, { origin, height, y, cuts, visible }: FlowTime, width: number): void {
  const elapsed = (time: number) => `+${d3.format(".3~f")((time - origin) / 1000)}s`;
  const ticks = y.ticks(Math.max(4, height / 150))
    .filter(at => !cuts.some(cut => y(at) > y(cut.start) - 19 && y(at) < y(cut.end) + 19));
  const values = [...ticks, ...cuts.flatMap(cut => [cut.start, cut.end])].sort(d3.ascending);
  const axis = svg.append("g").attr("class", "chart-axis grid flow-time-axis").attr("transform", "translate(74,0)");
  axis.call(d3.axisLeft(y).tickValues(values).tickFormat(time => elapsed(Number(time))).tickPadding(8).tickSize(-(width - 90)));
  element(axis, "path", { d: visible.map(({ start, end }) => `M0,${start}V${end}`).join(""), stroke: "var(--muted)", fill: "none" });
  for (const cut of cuts) drawAxisCut(axis, cut, y, elapsed);
}

function drawAxisCut(axis: Group, { start, end, kind }: FlowTime["cuts"][number], y: (at: number) => number, elapsed: (at: number) => string): void {
  const mark = axis.append("g").attr("class", "flow-axis-break").attr("data-start", start).attr("data-end", end);
  const label = kind === "input" ? "Between user prompts" : "Response wait";
  element(mark, "title", {}, `${label}, compressed from ${elapsed(start)} to ${elapsed(end)}`);
  element(mark, "path", { d: `M-6,${y(start) + 7}l4,-5 4,3 4,-5`, stroke: "var(--muted)", fill: "none" });
  element(mark, "path", { d: `M-6,${y(end)}l4,-5 4,3 4,-5`, stroke: "var(--muted)", fill: "none" });
}

function defineVisibleTimeClip(
  defs: d3.Selection<SVGDefsElement, unknown, null, undefined>,
  id: string,
  { visible }: FlowTime,
  width: number
): void {
  const clip = defs.append("clipPath").attr("id", id);
  for (const [index, section] of visible.entries()) {
    const start = index === 0 ? section.start - 2 : section.start;
    const end = index === visible.length - 1 ? section.end + 2 : section.end;
    element(clip, "rect", { x: 0, y: start, width, height: end - start });
  }
}

export function drawFlowGuides(svg: Svg, time: FlowTime, width: number, id: string): void {
  const { origin, timed } = time;
  const defs = svg.append("defs");
  defs.append("marker").attr("id", `${id}-return-arrow`).attr("viewBox", "0 0 10 10").attr("refX", 8).attr("refY", 5).attr("markerWidth", 6).attr("markerHeight", 6).attr("orient", "auto-start-reverse").append("path").attr("d", "M0,1 L9,5 L0,9").attr("fill", "var(--muted)");
  if (time.cuts.length) {
    defineVisibleTimeClip(defs, `${id}-visible-time`, time, width);
    svg.attr("aria-description", "Axis breaks shorten response waits and time between user prompts. Ticks show elapsed time from the UTC origin.");
  }
  element(svg, "text", { x: 20, y: 24, class: "chart-label" }, "Execution flow · time flows down");
  element(svg, "text", { x: 20, y: 44, class: "chart-label flow-origin" }, timed ? `UTC origin ${new Date(origin).toISOString()}` : "Source boundaries unavailable");
  drawFlowAxis(svg, time, width);
}
