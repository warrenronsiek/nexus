// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
// @boundary work-wire: validate typed projections and bounded observation evidence; the service validates lifecycle bodies.
import { decodeWorkDashboard, type WorkDashboard } from "./work-dashboard-wire.ts";
import type { NexusMcpClient } from "./mcp-client.ts";
import { boundedArray as array, isJsonValue, type JsonValue } from "./wire-values.ts";

export const WORK_OUTCOMES = ["succeeded", "failed", "canceled", "timed_out", "skipped", "infrastructure_error", "abandoned", "unknown"] as const;
export type WorkOutcome = typeof WORK_OUTCOMES[number];
export type WorkSubject = { scope: "session" | "activity"; id: string };
export type WorkValue = JsonValue;
interface GitRevision { repository: string; object_format: string; commit: string }
interface ExternalIdentity { provider: string; kind: string; id: string; url: string | null }
export interface WorkContext {
  host_session_id?: string | null; turn_id?: string | null; agent_instance_id?: string | null; model_id?: string | null;
  repository?: string | null; worktree?: string | null; environment?: string | null; revision?: GitRevision | null;
  external: ExternalIdentity[]; labels: Record<string, string>;
}
export interface WorkSummary {
  subject: WorkSubject; title: string; kind: string; work_session_ids: string[]; parent_activity_id: string | null;
  has_children: boolean; state: string; outcome: WorkOutcome | null; started_at: string | null; finished_at: string | null;
  elapsed: { nanoseconds: string; method: string; clock_id: string | null } | null;
  queued_at?: string | null; queue_elapsed?: { nanoseconds: string; method: string; clock_id: string | null } | null;
  conflicts: string[]; gaps: string[]; context: WorkContext;
}
export type WorkReference = { kind: "session" | "activity" | "eval"; id: string } |
  ({ kind: "revision" } & GitRevision) | { kind: "external"; provider: string; type: string; id: string; url: string | null };
export interface WorkLink { event_id: string; source: WorkSubject; relation: string; target: WorkReference; state: string }
interface WorkActor { agent: string; session_id: string; model_id: string | null; config_hash: string }
export interface WorkObservation {
  id: string; subject: WorkSubject; occurred_at: string; recorded_at: string; project_id: string; method: string;
  producer: { name: string; version: string; instance_id: string };
  body: { [key: string]: WorkValue; event: string };
  correction?: { replaces: string; reason: string } | null;
  source_clock?: { clock_id: string; nanoseconds: string } | null;
  provenance: WorkActor;
  source?: { project_id: string; recorded_at: string; provenance: WorkActor } | null;
}
export interface WorkQuery { environment?: string; repository?: string; host_session_id?: string; kind?: string; outcome?: WorkOutcome; search?: string;
  occurred_after?: string; occurred_before?: string; after?: string; limit?: number }
export interface WorkPage { items: WorkSummary[]; next_cursor: string | null }
export interface WorkTimeline extends WorkPage { work_session_id: string; parent_activity_id: string | null; unresolved_links: WorkLink[]; truncated: boolean }
export interface WorkDetail { summary: WorkSummary; observations: WorkObservation[]; links: WorkLink[]; truncated: boolean }
export interface WorkEvents { observations: WorkObservation[]; next_cursor: string | null }
export interface WorkCaptureHealth { available: boolean; queued?: number; rejected?: number; transport_rejected?: number; rejected_callbacks?: number; lost?: number; process_losses?: number; health_failures?: number; last_captured_at?: string | null; last_delivered_at?: string | null; legacy_capacity_losses?: number; rejected_evicted?: number; pending_callbacks?: number }
export interface WorkApi {
  health(signal?: AbortSignal): Promise<WorkCaptureHealth>;
  dashboard(project: string | null, sessionId?: string, signal?: AbortSignal): Promise<WorkDashboard>;
  activities(project: string | null, query: WorkQuery, signal?: AbortSignal): Promise<WorkPage>;
  list(project: string | null, query: WorkQuery, signal?: AbortSignal): Promise<WorkPage>;
  get(project: string | null, subject: WorkSubject, signal?: AbortSignal): Promise<WorkDetail | null>;
  timeline(project: string | null, session: string, parent: string | null, query: { after?: string; limit?: number }, signal?: AbortSignal): Promise<WorkTimeline>;
  events(project: string | null, subject: WorkSubject, query: { after?: string; limit?: number }, signal?: AbortSignal): Promise<WorkEvents>;
}
function object(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function texts(value: Record<string, unknown>, fields: string[]): boolean { return fields.every(field => typeof value[field] === "string"); }
function nullableText(value: unknown): value is string | null { return value === null || typeof value === "string"; }
function strings(value: unknown): value is string[] { return Array.isArray(value) && value.length <= 128 && value.every(item => typeof item === "string"); }
function subject(value: unknown): value is WorkSubject { return object(value) && (value.scope === "session" || value.scope === "activity") && typeof value.id === "string"; }
export function sameSubject(left: WorkSubject, right: WorkSubject): boolean { return left.scope === right.scope && left.id === right.id; }
function revision(value: unknown): value is GitRevision { return object(value) && texts(value, ["repository", "object_format", "commit"]); }
function external(value: unknown): value is ExternalIdentity { return object(value) && texts(value, ["provider", "kind", "id"]) && (value.url === undefined || nullableText(value.url)); }
function context(value: unknown): value is WorkContext {
  return object(value) && ["host_session_id", "turn_id", "agent_instance_id", "model_id", "repository", "worktree", "environment"].every(field => value[field] === undefined || nullableText(value[field])) &&
    (value.revision === undefined || value.revision === null || revision(value.revision)) && Array.isArray(value.external) && value.external.length <= 128 && value.external.every(external) &&
    object(value.labels) && Object.keys(value.labels).length <= 128 && Object.values(value.labels).every(item => typeof item === "string");
}
function elapsed(value: unknown): boolean {
  return value === null || (object(value) && texts(value, ["nanoseconds", "method"]) && /^\d+$/u.test(value.nanoseconds as string) && nullableText(value.clock_id));
}
function summary(value: unknown): value is WorkSummary {
  return object(value) && subject(value.subject) && texts(value, ["title", "kind", "state"]) && strings(value.work_session_ids) &&
    ["parent_activity_id", "started_at", "finished_at"].every(field => nullableText(value[field])) && typeof value.has_children === "boolean" && (value.outcome === null || WORK_OUTCOMES.some(item => item === value.outcome)) &&
    strings(value.conflicts) && strings(value.gaps) && context(value.context) &&
    elapsed(value.elapsed) && optional(value.queued_at, nullableText) && optional(value.queue_elapsed, elapsed);
}
function reference(value: unknown): value is WorkReference {
  if (!object(value)) return false;
  if (["session", "activity", "eval"].includes(String(value.kind))) return typeof value.id === "string";
  return value.kind === "revision" ? revision(value) : value.kind === "external" && texts(value, ["provider", "type", "id"]) && nullableText(value.url);
}
function link(value: unknown): value is WorkLink { return object(value) && texts(value, ["event_id", "relation", "state"]) && subject(value.source) && reference(value.target); }
function actor(value: unknown): value is WorkActor { return object(value) && texts(value, ["agent", "session_id", "config_hash"]) && nullableText(value.model_id); }
function optional(value: unknown, check: (value: unknown) => boolean): boolean { return value === undefined || value === null || check(value); }
function correction(value: unknown): boolean { return object(value) && texts(value, ["replaces", "reason"]); }
function sourceReceipt(value: unknown): boolean { return object(value) && texts(value, ["project_id", "recorded_at"]) && actor(value.provenance); }
function sourceClock(value: unknown): boolean { return object(value) && texts(value, ["clock_id", "nanoseconds"]) && /^\d+$/u.test(value.nanoseconds as string); }
function observation(value: unknown): value is WorkObservation {
  return object(value) && texts(value, ["id", "occurred_at", "recorded_at", "project_id", "method"]) && subject(value.subject) &&
    object(value.producer) && texts(value.producer, ["name", "version", "instance_id"]) && object(value.body) && typeof value.body.event === "string" && isJsonValue(value.body) &&
    optional(value.correction, correction) && actor(value.provenance) && optional(value.source, sourceReceipt) && optional(value.source_clock, sourceClock);
}
function page(value: Record<string, unknown>): value is Record<string, unknown> & WorkPage { return array(value.items, summary, 64) && nullableText(value.next_cursor); }
function detail(value: unknown): value is WorkDetail { return object(value) && summary(value.summary) && array(value.observations, observation, 64) && array(value.links, link) && typeof value.truncated === "boolean"; }
function invalid(): never { throw new Error("Nexus work history returned an unexpected response"); }
function nonnegativeCount(value: unknown): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0; }
function captureCounters(value: unknown): Pick<WorkCaptureHealth, "lost" | "transport_rejected" | "rejected_callbacks" | "last_captured_at" | "last_delivered_at"> | undefined {
  if (value === null) return { lost: undefined };
  if (!object(value) || !nonnegativeCount(value.lost) || ![value.last_captured_at, value.last_delivered_at].every(nullableText)) return;
  if (![value.transport_rejected, value.rejected_replays].every(item => item === undefined || nonnegativeCount(item))) return;
  return { lost: value.lost, transport_rejected: value.transport_rejected as number | undefined, rejected_callbacks: value.rejected_replays as number | undefined, last_captured_at: value.last_captured_at as string | null, last_delivered_at: value.last_delivered_at as string | null };
}
function captureRetention(health: Record<string, unknown>): Pick<WorkCaptureHealth, "legacy_capacity_losses" | "rejected_evicted" | "pending_callbacks"> | undefined {
  if (![health.dropped, health.rejected_evicted, health.pending_callbacks_process].every(value => value === undefined || nonnegativeCount(value))) return;
  return { legacy_capacity_losses: health.dropped as number | undefined, rejected_evicted: health.rejected_evicted as number | undefined, pending_callbacks: health.pending_callbacks_process as number | undefined };
}
function localCaptureHealth(value: unknown): WorkCaptureHealth | undefined {
  if (!object(value) || value.ok !== true || value.scope !== "local_companion" || typeof value.available !== "boolean") return;
  if (!value.available) return { available: false };
  const health = value.work_capture;
  if (!object(health) || ![health.queued, health.rejected, health.admission_lost_process, health.worker_failed_process, health.health_persistence_failed_process].every(nonnegativeCount)) return;
  const counters = captureCounters(health.capture), retention = captureRetention(health);
  if (!counters || !retention) return;
  return { available: true, queued: health.queued as number, rejected: health.rejected as number, ...counters, ...retention,
    process_losses: (health.admission_lost_process as number) + (health.worker_failed_process as number), health_failures: health.health_persistence_failed_process as number };
}

export class NexusWorkApi implements WorkApi {
  constructor(private readonly client: Pick<NexusMcpClient, "inspect">, private readonly cwd: string) {}
  async health(signal?: AbortSignal): Promise<WorkCaptureHealth> {
    const value = await this.client.inspect("nexus_capture_health", {}, this.cwd, signal);
    return localCaptureHealth(value) ?? invalid();
  }
  private async read(project: string | null, fields: Record<string, unknown>, response: string, signal?: AbortSignal): Promise<Record<string, unknown>> {
    const value = await this.client.inspect("nexus_work", { ...fields, ...(project === null ? {} : { project_id: project }) }, this.cwd, signal);
    if (!object(value) || value.ok !== true || value.response !== response) return invalid();
    return value;
  }
  private async subjects(scope: WorkSubject["scope"], project: string | null, query: WorkQuery, signal?: AbortSignal): Promise<WorkPage> {
    const value = await this.read(project, { action: scope === "session" ? "list" : "activities", query }, "page", signal);
    if (!page(value) || value.items.some(item => item.subject.scope !== scope)) return invalid();
    return { items: value.items, next_cursor: value.next_cursor };
  }
  list(project: string | null, query: WorkQuery, signal?: AbortSignal): Promise<WorkPage> {
    return this.subjects("session", project, query, signal);
  }
  activities(project: string | null, query: WorkQuery, signal?: AbortSignal): Promise<WorkPage> {
    return this.subjects("activity", project, query, signal);
  }
  async dashboard(project: string | null, sessionId?: string, signal?: AbortSignal): Promise<WorkDashboard> {
    const value = await this.read(project, { action: "dashboard", ...(sessionId ? { work_session_id: sessionId } : {}) }, "dashboard", signal);
    return decodeWorkDashboard(value, sessionId, summary) ?? invalid();
  }
  async get(project: string | null, wanted: WorkSubject, signal?: AbortSignal): Promise<WorkDetail | null> {
    const value = await this.read(project, { action: "get", subject: wanted }, "detail", signal);
    if (!(value.record === null || (detail(value.record) && sameSubject(value.record.summary.subject, wanted)))) return invalid();
    return value.record;
  }
  async timeline(project: string | null, session: string, parent: string | null, query: { after?: string; limit?: number }, signal?: AbortSignal): Promise<WorkTimeline> {
    const value = await this.read(project, { action: "timeline", work_session_id: session, ...(parent === null ? {} : { parent_activity_id: parent }), ...query }, "timeline", signal);
    if (!page(value) || value.work_session_id !== session || value.parent_activity_id !== parent || !array(value.unresolved_links, link) || typeof value.truncated !== "boolean" ||
      value.items.some(item => item.subject.scope !== "activity" || (parent !== null && item.parent_activity_id !== parent))) return invalid();
    return { items: value.items, next_cursor: value.next_cursor, work_session_id: session, parent_activity_id: parent, unresolved_links: value.unresolved_links, truncated: value.truncated };
  }
  async events(project: string | null, wanted: WorkSubject, query: { after?: string; limit?: number }, signal?: AbortSignal): Promise<WorkEvents> {
    const value = await this.read(project, { action: "events", subject: wanted, ...query }, "events", signal);
    if (!array(value.observations, observation, 64) || !nullableText(value.next_cursor) || value.observations.some(item => !sameSubject(item.subject, wanted))) return invalid();
    return { observations: value.observations, next_cursor: value.next_cursor };
  }
}
