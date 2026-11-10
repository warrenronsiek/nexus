// @feature code-map
// @feature observability-ui
// @spec docs/features/code-map.md
// @spec docs/features/observability-ui.md
import { hierarchy, treemap } from "d3";
import type { Choose, Tile } from "./chart-model";
import { actionable, element, type Svg } from "./chart-svg";

type Node = Tile | { children: Tile[] };

export function drawTreemap(svg: Svg, tiles: Tile[], width: number, choose: Choose, selected?: string | null): void {
  const height = Math.max(290, Math.min(410, Math.round(width * .55)));
  svg.attr("viewBox", `0 0 ${width} ${height}`).attr("width", width).attr("height", height);
  const root = hierarchy<Node>({ children: tiles.filter(tile => tile.weight > 0) })
    .sum(node => "weight" in node ? node.weight : 0);
  const leaves = treemap<Node>().size([width, height]).paddingInner(3).round(true)(root).leaves();
  for (const leaf of leaves) {
    if ("weight" in leaf.data) drawTile(svg, leaf.data, leaf, choose, selected);
  }
}

function drawTile(svg: Svg, tile: Tile, box: { x0: number; y0: number; x1: number; y1: number }, choose: Choose, selected?: string | null): void {
  const width = box.x1 - box.x0, height = box.y1 - box.y0;
  const group = svg.append("g").attr("class", "code-map-tile")
    .attr("transform", `translate(${box.x0},${box.y0})`)
    .attr("aria-label", `${tile.label}, ${tile.value}`)
    .attr("data-selected", String(selected === tile.id));
  actionable(group, tile.id, choose);
  element(group, "title", {}, `${tile.label} · ${tile.value}`);
  element(group, "rect", { width, height, rx: 2 });
  const characters = Math.floor((width - 16) / 8);
  if (characters < 4 || height < 24) return;
  const label = tile.label.length <= characters ? tile.label : tile.label.slice(0, characters - 1) + "…";
  element(group, "text", { x: 8, y: 18, class: "chart-label" }, label);
  if (height >= 46 && tile.value.length <= characters) {
    element(group, "text", { x: 8, y: 36, class: "chart-value" }, tile.value);
  }
}
