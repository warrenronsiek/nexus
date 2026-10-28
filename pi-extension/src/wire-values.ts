// @feature observability-ui
// @feature evals
// @feature work-history
// @feature code-concerns
// @spec docs/features/observability-ui.md
// @spec docs/features/evals.md
// @spec docs/features/work-history.md
// @spec docs/features/code-concerns.md
// @boundary bounded-json-evidence: share finite, bounded JSON evidence validation across history panes.
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type FieldGuard = (value: unknown) => boolean;
export type FieldShape = Record<string, FieldGuard>;
/** Named wire fields remain explicit; domain guards supply their own invariants. */
export function hasShape(value: unknown, shape: FieldShape): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value) &&
    Object.entries(shape).every(([field, guard]) => field in value && guard((value as Record<string, unknown>)[field]));
}
export function boundedArray<T>(value: unknown, check: (item: unknown) => item is T, maximum = 128): value is T[] {
  return Array.isArray(value) && value.length <= maximum && value.every(check);
}
function visit(value: unknown, depth: number, budget: { remaining: number }): value is JsonValue {
  if (depth > 32 || --budget.remaining < 0) return false;
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(item => visit(item, depth + 1, budget));
  return typeof value === "object" && Object.values(value).every(item => visit(item, depth + 1, budget));
}
export function isJsonValue(value: unknown): value is JsonValue { return visit(value, 0, { remaining: 16384 }); }
