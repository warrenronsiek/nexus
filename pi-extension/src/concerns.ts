// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
// @boundary concern-wire: validate bounded concern projections and literal observation evidence before presentation.
import type { NexusMcpClient } from "./mcp-client.ts";
import { boundedArray as array, hasShape, isJsonValue, type JsonValue } from "./wire-values.ts";

export type ConcernSort = "recent" | "work_sessions" | "returns";
export interface ConcernQuery { repository?: string | null; area_id?: string | null; status?: "open" | "resolved" | null; search?: string | null; occurred_after?: string | null; occurred_before?: string | null; sort?: ConcernSort; after?: string | null; limit?: number | null }
export interface ConcernCounts { raw_observations: number; known_source_occurrences: number; work_sessions: number; host_sessions: number; unknown_source_observations: number; unknown_work_observations: number; unknown_host_observations: number; unknown_source_time_observations: number; concerns: number; returns_after_resolution: number }
export interface ConcernArea { id: string; repository: string | null; kind: "feature" | "subsystem" | "file" | "declaration"; key: string; display_name: string; discriminator: string | null }
export interface ConcernAreaSummary { area: ConcernArea; counts: ConcernCounts; open_concern_count: number; last_observed_at: string | null }
export interface ConcernDaily { date: string; raw_observations: number; known_source_occurrences: number; work_sessions: number }
export interface ConcernDashboard { project_id: string; query: ConcernQuery; counts: ConcernCounts; areas: ConcernAreaSummary[]; daily: ConcernDaily[]; next_cursor: string | null; partial: boolean; notices: string[] }
export interface ConcernSummary { id: string; canonical_id: string; title: string; description: string; hypothesis: string | null; impact: string | null; status: "open" | "resolved" | "consolidated"; counts: ConcernCounts; last_observed_at: string | null; created_at: string; resolved_at: string | null; lifecycle_qualification: string | null; areas: ConcernArea[]; areas_partial?: boolean }
export interface ConcernContext { host_session_id: string | null; work_session_id: string | null; activity_id: string | null; repository: string | null }
export interface ConcernObservation { input: { id: string; concern_id: string; occurred_at: string | null; observed_at: string; context: ConcernContext; body: { event: string; [key: string]: JsonValue } }; project_id: string; recorded_at: string; provenance: { agent: string; session_id: string; model_id: string | null; config_hash: string }; source?: JsonValue }
export interface ConcernSessionTitle { agent: string; host_session_id: string | null; work_session_id: string | null; host_title: string | null; work_title: string | null }
export interface ConcernDetail { summary: ConcernSummary; history: ConcernObservation[]; next_cursor: string | null; partial: boolean; session_titles?: ConcernSessionTitle[] }
export interface ConcernPage { project_id: string; query: ConcernQuery; items: ConcernSummary[]; next_cursor: string | null; total: number; partial: boolean }
export interface ConcernHistory { concern_id: string; observations: ConcernObservation[]; next_cursor: string | null; session_titles?: ConcernSessionTitle[] }
export interface ConcernsApi {
  dashboard(project: string | null, query: ConcernQuery, signal?: AbortSignal): Promise<ConcernDashboard>;
  list(project: string | null, query: ConcernQuery, signal?: AbortSignal): Promise<ConcernPage>;
  get(project: string | null, concern: string, signal?: AbortSignal): Promise<ConcernDetail | null>;
  history(project: string | null, concern: string, after?: string, signal?: AbortSignal): Promise<ConcernHistory>;
}
function object(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function texts(value: Record<string, unknown>, fields: string[]): boolean { return fields.every(field => typeof value[field] === "string"); }
function nullable(value: unknown): value is string | null { return value === null || typeof value === "string"; }
function count(value: unknown): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0; }
const COUNT_FIELDS = ["raw_observations", "known_source_occurrences", "work_sessions", "host_sessions", "unknown_source_observations", "unknown_work_observations", "unknown_host_observations", "unknown_source_time_observations", "concerns", "returns_after_resolution"];
function counts(value: unknown): value is ConcernCounts { return object(value) && COUNT_FIELDS.every(field => count(value[field])); }
function area(value: unknown): value is ConcernArea { return object(value) && texts(value, ["id", "key", "display_name"]) && nullable(value.repository) && nullable(value.discriminator) && ["feature", "subsystem", "file", "declaration"].includes(String(value.kind)); }
function areaSummary(value: unknown): value is ConcernAreaSummary { return object(value) && area(value.area) && counts(value.counts) && count(value.open_concern_count) && nullable(value.last_observed_at); }
function daily(value: unknown): value is ConcernDaily { return object(value) && typeof value.date === "string" && /^\d{4}-\d{2}-\d{2}$/u.test(value.date) && [value.raw_observations, value.known_source_occurrences, value.work_sessions].every(count); }
function summary(value: unknown): value is ConcernSummary {
  return object(value) && texts(value, ["id", "canonical_id", "title", "description", "created_at"]) && ["open", "resolved", "consolidated"].includes(String(value.status)) &&
    ["hypothesis", "impact", "last_observed_at", "resolved_at", "lifecycle_qualification"].every(field => nullable(value[field])) && counts(value.counts) && array(value.areas, area, 128) && (value.areas_partial === undefined || typeof value.areas_partial === "boolean");
}
function observation(value: unknown): value is ConcernObservation {
  if (!object(value) || !texts(value, ["project_id", "recorded_at"]) || !object(value.input) || !object(value.provenance)) return false;
  const input = value.input, context = input.context, provenance = value.provenance;
  return texts(input, ["id", "concern_id", "observed_at"]) && nullable(input.occurred_at) && object(context) && ["host_session_id", "work_session_id", "activity_id", "repository"].every(field => nullable(context[field])) &&
    object(input.body) && typeof input.body.event === "string" && isJsonValue(input.body) && texts(provenance, ["agent", "session_id", "config_hash"]) && nullable(provenance.model_id) && (value.source === undefined || isJsonValue(value.source));
}
function detail(value: unknown): value is ConcernDetail {
  return hasShape(value, { summary, history: value => array(value, observation, 32), next_cursor: nullable, partial: value => typeof value === "boolean" }) &&
    (value.session_titles === undefined || array(value.session_titles, sessionTitle, 32));
}
function sessionTitle(value: unknown): value is ConcernSessionTitle { return object(value) && texts(value, ["agent"]) && ["host_session_id", "work_session_id", "host_title", "work_title"].every(field => nullable(value[field])); }
function query(value: unknown): value is ConcernQuery {
  return object(value) && isJsonValue(value) && ["repository", "area_id", "search", "occurred_after", "occurred_before", "after"].every(field => value[field] === undefined || nullable(value[field])) &&
    (value.sort === undefined || ["recent", "work_sessions", "returns"].includes(String(value.sort))) &&
    (value.status === undefined || value.status === null || ["open", "resolved"].includes(String(value.status))) &&
    (value.limit === undefined || value.limit === null || count(value.limit));
}
function scoped(value: Record<string, unknown>, project: string | null): boolean { return typeof value.project_id === "string" && (project === null || value.project_id === project) && query(value.query) && nullable(value.next_cursor) && typeof value.partial === "boolean"; }
function invalid(): never { throw new Error("Nexus code concerns returned an unexpected response"); }

export class NexusConcernsApi implements ConcernsApi {
  constructor(private readonly client: Pick<NexusMcpClient, "inspect">, private readonly cwd: string) {}
  private async read(project: string | null, fields: Record<string, unknown>, response: string, signal?: AbortSignal): Promise<Record<string, unknown>> {
    const value = await this.client.inspect("nexus_concerns", { ...fields, ...(project === null ? {} : { project_id: project }) }, this.cwd, signal);
    if (!object(value) || value.ok !== true || value.response !== response) return invalid();
    const { ok: _ok, response: _response, ...data } = value;
    return data;
  }
  async dashboard(project: string | null, query: ConcernQuery, signal?: AbortSignal): Promise<ConcernDashboard> {
    const value = await this.read(project, { action: "dashboard", query }, "dashboard", signal);
    return scoped(value, project) && counts(value.counts) && array(value.areas, areaSummary, 64) && array(value.daily, daily, 4096) && array(value.notices, (item): item is string => typeof item === "string", 128) ? value as unknown as ConcernDashboard : invalid();
  }
  async list(project: string | null, query: ConcernQuery, signal?: AbortSignal): Promise<ConcernPage> {
    const value = await this.read(project, { action: "list", query }, "page", signal);
    return scoped(value, project) && array(value.items, summary, 64) && count(value.total) ? value as unknown as ConcernPage : invalid();
  }
  async get(project: string | null, concern_id: string, signal?: AbortSignal): Promise<ConcernDetail | null> {
    const value = await this.read(project, { action: "get", concern_id }, "detail", signal);
    return value.concern === null || detail(value.concern) ? value.concern as ConcernDetail | null : invalid();
  }
  async history(project: string | null, concern_id: string, after?: string, signal?: AbortSignal): Promise<ConcernHistory> {
    const value = await this.read(project, { action: "history", concern_id, ...(after ? { after } : {}), limit: 32 }, "history", signal);
    return value.concern_id === concern_id && array(value.observations, observation, 32) && nullable(value.next_cursor) && (value.session_titles === undefined || array(value.session_titles, sessionTitle, 32)) ? value as unknown as ConcernHistory : invalid();
  }
}
