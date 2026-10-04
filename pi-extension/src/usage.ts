// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
import type { UsageItem, UsageSummary } from "./domain.ts";

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0;
}

function isOptionalCount(value: unknown): boolean {
  return value === undefined || isNonNegativeInteger(value);
}

function isEvidence(value: unknown): boolean {
  return (
    value === undefined ||
    typeof value === "string" ||
    (isObject(value) && Object.values(value).every(isNonNegativeInteger))
  );
}

function isUsageItem(value: unknown): value is UsageItem {
  return (
    isObject(value) &&
    (value.kind === "tool" || value.kind === "script" || value.kind === "skill") &&
    typeof value.name === "string" &&
    value.name.length > 0 &&
    isNonNegativeInteger(value.count) &&
    isNonNegativeInteger(value.sessions) &&
    isOptionalCount(value.succeeded) &&
    isOptionalCount(value.failed) &&
    isOptionalCount(value.observed) &&
    isEvidence(value.evidence)
  );
}

export function decodeUsage(value: unknown): UsageSummary {
  if (
    !isObject(value) ||
    value.ok !== true ||
    !(value.project_id === null || typeof value.project_id === "string") ||
    typeof value.window_started_at !== "string" ||
    typeof value.window_ended_at !== "string" ||
    !Array.isArray(value.tools) ||
    !value.tools.every(isUsageItem) ||
    !Array.isArray(value.skills) ||
    !value.skills.every(isUsageItem) ||
    !isObject(value.capture_health)
  ) {
    throw new Error("Nexus usage analytics returned an unexpected response");
  }
  return value as unknown as UsageSummary;
}

function trimToWidth(value: string, width: number): string {
  if (width <= 0) return "";
  if (value.length <= width) return value;
  if (width === 1) return "…";
  return `${value.slice(0, width - 1)}…`;
}

function usageLabel(item: UsageItem): string {
  return item.kind === "script" ? `[script] ${item.name}` : item.name;
}

function usageMetric(item: UsageItem): string {
  const sessionLabel = item.sessions === 1 ? "session" : "sessions";
  return `${item.count} · ${item.sessions} ${sessionLabel}`;
}

interface BarLayout {
  availableWidth: number;
  barWidth: number;
  labelWidth: number;
  maximum: number;
  metricWidth: number;
}

function barLayout(
  items: readonly UsageItem[],
  metrics: readonly string[],
  availableWidth: number,
): BarLayout {
  const metricWidth = Math.max(...metrics.map((metric) => metric.length));
  const naturalLabelWidth = Math.max(...items.map((item) => usageLabel(item).length));
  const chartWidth = Math.max(1, availableWidth - metricWidth - 2);
  const labelWidth = Math.min(naturalLabelWidth, Math.max(4, chartWidth - 2));
  return {
    availableWidth,
    barWidth: Math.max(1, chartWidth - labelWidth - 1),
    labelWidth,
    maximum: Math.max(...items.map((item) => item.count), 1),
    metricWidth,
  };
}

function renderUsageBar(item: UsageItem, metric: string, layout: BarLayout): string {
  const label = trimToWidth(usageLabel(item), layout.labelWidth).padEnd(layout.labelWidth);
  const proportionalWidth = Math.round((item.count / layout.maximum) * layout.barWidth);
  const filled = item.count === 0 ? 0 : Math.max(1, proportionalWidth);
  const bar = "█".repeat(filled).padEnd(layout.barWidth);
  return trimToWidth(
    `${label} ${bar} ${metric.padStart(layout.metricWidth)}`,
    layout.availableWidth,
  );
}

export function renderUsageBars(items: readonly UsageItem[], width: number): string[] {
  const availableWidth = Math.max(1, Math.floor(width));
  if (items.length === 0) {
    return [trimToWidth("No usage observed in this window.", availableWidth)];
  }

  const metrics = items.map(usageMetric);
  const layout = barLayout(items, metrics, availableWidth);
  return items.map((item, index) => renderUsageBar(item, metrics[index], layout));
}
