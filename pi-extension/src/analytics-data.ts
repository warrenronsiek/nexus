// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @boundary analytics-json
export interface Activity {
  category_id: string; name: string; calls: number; actions: number; sessions: number; improvised_actions: number;
}
export interface Capability {
  resource_key: string; name: string; kind: string; origin: string; calls: number;
  offered_sessions: number; discoverable_sessions: number; availability: string; description: string;
}
export interface Opportunity {
  category_id: string; name: string; calls: number; sessions: number; supplied_alternatives: string[]; evidence: string;
}
export interface Example {
  name: string; description: string; origin: string; evidence: string; tool_name: string; call_outcome: string;
  model_id: string | null; observed_at: string; category_id: string;
}
export interface Catalog { ok: true; capabilities: Capability[]; next_offset: number | null }
export interface Examples { ok: true; examples: Example[]; next_offset: number | null }
export interface ExampleFilter { category_id?: string; resource_key?: string; offset: number }
export interface AnalyticsBundle {
  summary: { ok: true; observed_calls: number; identified_actions: number; classified_actions: number; unclassified_actions: number;
    activities: Activity[]; truncated: boolean; window_started_at: string; window_ended_at: string; counting: string };
  catalog: Catalog;
  opportunities: { ok: true; opportunities: Opportunity[]; truncated: boolean };
}
export interface AnalyticsApi {
  catalog(project: string | null, offset: number, signal: AbortSignal): Promise<Catalog>;
  examples(project: string | null, filter: ExampleFilter, signal: AbortSignal): Promise<Examples>;
}
function object(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Invalid analytics object");
  return value as Record<string, unknown>;
}
function count(value: unknown): boolean { return Number.isSafeInteger(value) && Number(value) >= 0; }
function fields(value: unknown, strings: string[], counts: string[] = []): Record<string, unknown> {
  const row = object(value);
  if (!strings.every((key) => typeof row[key] === "string") || !counts.every((key) => count(row[key]))) throw new Error("Invalid analytics fields");
  return row;
}
function list(value: unknown, validate: (row: unknown) => void, maximum = 64): void {
  if (!Array.isArray(value) || value.length > maximum) throw new Error("Invalid analytics list");
  value.forEach(validate);
}
function page(value: unknown): Record<string, unknown> {
  const row = object(value);
  if (row.ok !== true || !(row.next_offset === null || count(row.next_offset))) throw new Error("Invalid analytics page");
  return row;
}
function activity(value: unknown): void { fields(value, ["category_id", "name"], ["calls", "actions", "sessions", "improvised_actions"]); }
function capability(value: unknown): void {
  fields(value, ["resource_key", "name", "kind", "origin", "availability", "description"], ["calls", "offered_sessions", "discoverable_sessions"]);
}
function opportunity(value: unknown): void {
  const row = fields(value, ["category_id", "name", "evidence"], ["calls", "sessions"]);
  list(row.supplied_alternatives, (name) => { if (typeof name !== "string") throw new Error("Invalid alternative"); }, 256);
}
export function decodeCatalog(value: unknown): Catalog { const row = page(value); list(row.capabilities, capability); return value as Catalog; }
export function decodeExamples(value: unknown): Examples {
  const row = page(value);
  list(row.examples, (value) => {
    const item = fields(value, ["name", "description", "origin", "evidence", "tool_name", "call_outcome", "observed_at", "category_id"]);
    if (!(item.model_id === null || typeof item.model_id === "string")) throw new Error("Invalid classification model");
  }, 32);
  return value as Examples;
}
export function decodeAnalytics(value: unknown): AnalyticsBundle {
  const bundle = object(value);
  const summary = fields(bundle.summary, ["window_started_at", "window_ended_at", "counting"], ["observed_calls", "identified_actions", "classified_actions", "unclassified_actions"]);
  const opportunities = object(bundle.opportunities);
  if (summary.ok !== true || opportunities.ok !== true || typeof summary.truncated !== "boolean" || typeof opportunities.truncated !== "boolean") throw new Error("Invalid analytics summary");
  list(summary.activities, activity);
  list(opportunities.opportunities, opportunity);
  decodeCatalog(bundle.catalog);
  return value as AnalyticsBundle;
}
