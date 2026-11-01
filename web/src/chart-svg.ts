// @feature observability-ui
// @spec docs/features/observability-ui.md
import * as d3 from "d3";
import type { Choose } from "./chart-model";

export type Svg = d3.Selection<SVGSVGElement, unknown, null, undefined>;
export type Group = d3.Selection<SVGGElement, unknown, null, undefined>;
type Attrs = Record<string, string | number | null>;

export function attrs<E extends SVGElement>(selection: d3.Selection<E, unknown, null, undefined>, values: Attrs): void {
  Object.entries(values).forEach(([key, value]) => selection.attr(key, value));
}
export function element<E extends SVGElement>(parent: d3.Selection<E, unknown, null, undefined>, tag: string, values: Attrs, text?: string): void {
  const node = parent.append<SVGElement>(tag);
  attrs(node, values);
  if (text !== undefined) node.text(text);
}
export function actionable<E extends SVGElement>(selection: d3.Selection<E, unknown, null, undefined>, id: string, choose: Choose): void {
  selection.attr("tabindex", 0).attr("role", "button").on("click", () => choose(id));
  selection.on("keydown", event => {
    const key = (event as KeyboardEvent).key;
    if (key === "Enter" || key === " ") { event.preventDefault(); choose(id); }
  });
}
export function reveal(host: HTMLElement): void {
  const scroll = host.closest<HTMLElement>(".waterfall-scroll"), selected = host.querySelector<SVGGElement>(".waterfall-node.selected");
  if (!scroll || !selected || typeof selected.getBBox !== "function") return;
  const bounds = selected.getBBox();
  if (bounds.y < scroll.scrollTop || bounds.y > scroll.scrollTop + scroll.clientHeight - 60) scroll.scrollTop = Math.max(0, bounds.y - 100);
  if (bounds.x < scroll.scrollLeft || bounds.x > scroll.scrollLeft + scroll.clientWidth - 100) scroll.scrollLeft = Math.max(0, bounds.x - 100);
}
