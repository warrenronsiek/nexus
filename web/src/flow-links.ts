// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import { type Box } from "./flow-layout";
interface Link { left: number; right: number; direction: number; at: number }
type Point = [number, number];

/** Transport links detour around labels; their source-time endpoints stay fixed. */
export function flowLink(from: number, to: number, at: number, boxes: Box[]): string {
  const link: Link = { left: Math.min(from, to), right: Math.max(from, to), direction: Math.sign(to - from), at };
  const points: Point[] = [[from, at]];
  for (const box of blockers(boxes, link)) points.push(...detour(box, boxes, link));
  points.push([to, at]);
  return d3.line()(points)!;
}
function blockers(boxes: Box[], link: Link): Box[] {
  return boxes.filter(box => box.left < link.right && box.right > link.left && box.top <= link.at && box.bottom >= link.at)
    .sort((a, b) => link.direction * (a.left - b.left));
}
function clearance(box: Box, boxes: Box[], at: number): number {
  const candidates = [box.top - 8, box.bottom + 8].sort((a, b) => Math.abs(a - at) - Math.abs(b - at));
  return candidates.find(y => !boxes.some(other => other.left < box.right && other.right > box.left && other.top < y && other.bottom > y)) ?? candidates[0];
}
function detour(box: Box, boxes: Box[], link: Link): Point[] {
  const y = clearance(box, boxes, link.at);
  const before = link.direction > 0 ? Math.max(link.left, box.left - 8) : Math.min(link.right, box.right + 8);
  const after = link.direction > 0 ? Math.min(link.right, box.right + 8) : Math.max(link.left, box.left - 8);
  return [[before, link.at], [before, y], [after, y], [after, link.at]];
}
