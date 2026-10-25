// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
// @boundary dynamic-json: validate bounded dashboard projections without interpreting exact metric values or choosing optimization goals.
export interface EvalSet {
  id: string; label: string; suite_identity: string; suite_revision_ids: string[];
  target_identity: string; target_revision_ids: string[]; components: string[];
  repository: string[]; models: string[]; latest_at: string | null;
  statuses: Record<string, number>; run_count: number; result_count: number;
}
export type EvalMeasurement = { storage: "inline"; text: string } | { storage: "artifact"; artifact_id: string } | { storage: "unavailable"; reason: string };
export interface EvalPoint {
  record_id: string; run_id: string; suite_revision_id: string; target_revision_id: string; occurred_at: string; value: EvalMeasurement;
  numeric_value: number | null; replaces: string | null;
}
export interface EvalSeries {
  id: string; label: string; unit: string; definition_id: string; description: string;
  calculation: string; subject_scope: string; points: EvalPoint[]; superseded_count: number;
}
export interface EvalDashboard {
  groups: EvalSet[]; selected_group_id: string | null; series: EvalSeries[];
  run_count: number; result_count: number; scanned_records: number; source_limit: number; truncated: boolean; notices: string[];
}
function object(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function texts(value: Record<string, unknown>, fields: string[]): boolean { return fields.every(field => typeof value[field] === "string"); }
function nullable(value: unknown): value is string | null { return value === null || typeof value === "string"; }
function count(value: unknown): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0; }
function strings(value: unknown, limit = 4096): value is string[] { return Array.isArray(value) && value.length <= limit && value.every(item => typeof item === "string"); }
function group(value: unknown): value is EvalSet {
  return object(value) && texts(value, ["id", "label", "suite_identity", "target_identity"]) &&
    ["suite_revision_ids", "target_revision_ids", "components", "repository", "models"].every(field => strings(value[field])) &&
    nullable(value.latest_at) && count(value.run_count) && count(value.result_count) &&
    object(value.statuses) && Object.values(value.statuses).every(count);
}
function measurement(value: unknown): value is EvalMeasurement {
  if (!object(value)) return false;
  switch (value.storage) {
    case "inline": return typeof value.text === "string";
    case "artifact": return typeof value.artifact_id === "string";
    case "unavailable": return typeof value.reason === "string";
    default: return false;
  }
}
function point(value: unknown): value is EvalPoint {
  return object(value) && texts(value, ["record_id", "run_id", "suite_revision_id", "target_revision_id", "occurred_at"]) && measurement(value.value) && nullable(value.replaces) &&
    (value.numeric_value === null || (typeof value.numeric_value === "number" && Number.isFinite(value.numeric_value)));
}
function series(value: unknown): value is EvalSeries {
  return object(value) && texts(value, ["id", "label", "unit", "definition_id", "description", "calculation", "subject_scope"]) && count(value.superseded_count) &&
    Array.isArray(value.points) && value.points.length <= 512 && value.points.every(point);
}
function bounded<T>(value: unknown, limit: number, guard: (item: unknown) => item is T): value is T[] {
  return Array.isArray(value) && value.length <= limit && value.every(guard);
}
function dashboard(value: unknown): value is EvalDashboard {
  return object(value) && value.ok === true && value.action === "dashboard" && nullable(value.selected_group_id) &&
    ["run_count", "result_count", "scanned_records", "source_limit"].every(field => count(value[field])) &&
    typeof value.truncated === "boolean" && strings(value.notices) && bounded(value.groups, 512, group) && bounded(value.series, 64, series);
}
function selectionMatches(value: EvalDashboard, requested: string | undefined): boolean {
  const selected = value.selected_group_id;
  return (requested === undefined || selected === requested) && (selected === null || value.groups.some(group => group.id === selected));
}
export function decodeEvalDashboard(value: unknown, requested?: string): EvalDashboard {
  if (!dashboard(value) || !selectionMatches(value, requested)) throw new Error("Nexus eval history returned an unexpected response");
  return value;
}
