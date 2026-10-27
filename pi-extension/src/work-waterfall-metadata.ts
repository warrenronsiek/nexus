// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { activityStyle } from "./work-activity-style.ts";
import type { WaterfallNode } from "./work-waterfall.ts";

/** Keep source values literal; shared terminal rendering owns control sanitation. */
function preview(value: string | null | undefined, maximum = 120): string {
  if (value === null || value === undefined) return "unknown";
  return value.length <= maximum ? value : `${value.slice(0, maximum)}… [display omitted]`;
}
function duration(value: WaterfallNode["elapsed"]): string {
  if (!value) return "unknown";
  if (value.nanoseconds.length > 80) return "unknown [duration exceeds display bound]";
  const digits = value.nanoseconds.padStart(10, "0");
  const exact = `${digits.slice(0, -9).replace(/^0+(?=\d)/u, "")}.${digits.slice(-9)}s`;
  return `${exact} · ${preview(value.method, 64)} · clock ${preview(value.clock_id, 96)}`;
}
function notices(values: string[]): string {
  return values.length ? `${values.slice(0, 2).map(value => preview(value, 96)).join(", ")}${values.length > 2 ? ` · +${values.length - 2} omitted` : ""}` : "none reported";
}
/** Bounded selected-node metadata, independent of immutable observation retrieval. */
export function waterfallMetadata(node: WaterfallNode, scope: "session" | "activity"): string[] {
  const parent = node.parent_activity_id === null ? scope === "session" ? "none (session)" : "session root" : preview(node.parent_activity_id, 192);
  return [
    `Selected: ${preview(node.title)}`,
    `Kind: ${activityStyle(node.kind).label} (${preview(node.kind, 64)}) · state ${preview(node.state, 32)} · outcome ${node.outcome ?? "unknown"}`,
    `Identity: ${scope}:${preview(node.id, 192)} · parent ${parent}`,
    `Source: start ${preview(node.started_at, 96)} · finish ${preview(node.finished_at, 96)}`,
    `Elapsed: ${duration(node.elapsed)}`,
    `Queue: ${preview(node.queued_at, 96)} · wait ${duration(node.queue_elapsed ?? null)}`,
    `Executor: ${preview(node.executor, 192)} · accounting ${node.accounting_role ?? "unknown"}`,
    `Gaps: ${notices(node.gaps)} · conflicts: ${notices(node.conflicts)}`,
  ];
}
