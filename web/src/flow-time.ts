// @feature observability-ui
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/work-history.md
import * as d3 from "d3";
import type { Span } from "./chart-model";
import { callerWait, qualified } from "./flow-state";

const top = 116;
const bottom = 48;
const cutHeight = 32;
const waitPadding = 24;
const minimumWaitHeight = 96;
interface Interval { start: number; end: number }
interface Cut extends Interval { kind: "input" | "response" }
interface WaitChanges { input: Map<number, number>; response: Map<number, number> }

function addWait(changes: Map<number, number>, { start, end }: Interval): void {
  if (start >= end) return;
  changes.set(start, (changes.get(start) ?? 0) + 1);
  changes.set(end, (changes.get(end) ?? 0) - 1);
}

function promptGaps(prompts: Span[], changes: Map<number, number>): void {
  if (prompts.some(prompt => prompt.start === null || prompt.conflicts?.length)) return;
  let latestResponse: number | null = null;
  for (const prompt of prompts.sort((a, b) => a.start! - b.start!)) {
    if (latestResponse !== null) addWait(changes, { start: latestResponse, end: prompt.start! });
    if (!qualified(prompt)) return;
    latestResponse = Math.max(latestResponse ?? prompt.end!, prompt.end!);
  }
}

function boundaries(spans: Span[]): number[] {
  const times = spans.flatMap(span => [span.queued, span.start, span.end, span.returnedAt ?? null]);
  return [...new Set(times.filter((time): time is number => time !== null))].sort(d3.ascending);
}

function waitChanges(spans: Span[]): WaitChanges {
  const byId = new Map(spans.map(span => [span.id, span]));
  const requests = new Map<string, Span[]>();
  const response = new Map<number, number>(), input = new Map<number, number>();
  for (const span of spans) {
    const parent = byId.get(span.parent ?? "");
    const waits = [callerWait(span, parent)];
    if (span.role === "wait" && qualified(span)) waits.push({ start: span.start!, end: span.end!, state: "wait" });
    for (const wait of waits) {
      if (wait) addWait(response, wait);
    }
    if (span.kind === "request" && parent?.kind === "host.session") {
      const prompts = requests.get(parent.id) ?? [];
      prompts.push(span);
      requests.set(parent.id, prompts);
    }
  }
  for (const prompts of requests.values()) promptGaps(prompts, input);
  return { input, response };
}

/** A cut stays inside observed waiting and keeps every recorded event on the visible scale. */
function waitCuts(times: number[], changes: WaitChanges, pixelsPerMs: number): Cut[] {
  const cuts: Cut[] = [];
  const padding = Math.ceil(waitPadding / pixelsPerMs);
  let input = 0, response = 0;
  for (const [index, start] of times.slice(0, -1).entries()) {
    input += changes.input.get(start) ?? 0;
    response += changes.response.get(start) ?? 0;
    const end = times[index + 1];
    if ((input > 0 || response > 0) && (end - start) * pixelsPerMs >= minimumWaitHeight) {
      const cut = { start: start + padding, end: end - padding };
      if ((cut.end - cut.start) * pixelsPerMs > cutHeight) cuts.push({ ...cut, kind: input > 0 ? "input" : "response" });
    }
  }
  return cuts;
}

export function flowTime(spans: Span[], zoom: number) {
  const times = boundaries(spans);
  const origin = times[0] ?? 0, horizon = times.at(-1) ?? origin;
  const limit = Math.max(origin + 1, horizon);
  const readableHeight = Math.max(1000, spans.length * 40);
  const available = Math.max(360, readableHeight * zoom) - top - bottom;
  const fitPixelsPerMs = Math.min(available, readableHeight - top - bottom) / (limit - origin);
  const cuts = waitCuts(times, waitChanges(spans), fitPixelsPerMs);
  return { origin, horizon, cuts, timed: times.length > 0, ...projectCuts(origin, limit, available, cuts) };
}

function projectCuts(origin: number, limit: number, available: number, cuts: Interval[]) {
  const domain = [origin], range = [top];
  const pixelsPerMs = available / (limit - origin);
  for (const cut of cuts) {
    const startY = range.at(-1)! + (cut.start - domain.at(-1)!) * pixelsPerMs;
    domain.push(cut.start, cut.end);
    range.push(startY, startY + cutHeight);
  }
  const endY = cuts.length ? range.at(-1)! + (limit - domain.at(-1)!) * pixelsPerMs : top + available;
  domain.push(limit);
  range.push(endY);
  const visible = range.flatMap((start, index) => index % 2 === 0 ? [{ start, end: range[index + 1] }] : []);
  return { height: endY + bottom, y: d3.scaleLinear(domain, range), visible };
}

export type FlowTime = ReturnType<typeof flowTime>;
