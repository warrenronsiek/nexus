// @feature observability-ui
// @feature evals
// @feature work-history
// @spec docs/features/observability-ui.md
// @spec docs/features/evals.md
// @spec docs/features/work-history.md
// @boundary bounded-json-evidence: share finite, bounded JSON evidence validation across history panes.
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
function visit(value: unknown, depth: number, budget: { remaining: number }): value is JsonValue {
  if (depth > 32 || --budget.remaining < 0) return false;
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(item => visit(item, depth + 1, budget));
  return typeof value === "object" && Object.values(value).every(item => visit(item, depth + 1, budget));
}
export function isJsonValue(value: unknown): value is JsonValue { return visit(value, 0, { remaining: 16384 }); }
