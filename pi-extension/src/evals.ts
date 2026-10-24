// @feature evals
// @spec docs/features/evals.md
// @feature observability-ui
// @spec docs/features/observability-ui.md
// @boundary dynamic-json: validate the history envelope and JSON values; the service owns typed eval body validation.
import type { NexusMcpClient } from "./mcp-client.ts";
import { isJsonValue, type JsonValue } from "./wire-values.ts";

export const EVAL_KINDS = ["change", "repository", "model", "case", "suite", "grader", "target", "experiment", "run", "trial", "attempt", "call", "trace", "grade", "comparison", "decision", "selection", "artifact", "verification", "lifecycle", "metric", "measurement"] as const;
export type EvalKind = typeof EVAL_KINDS[number];
const REPORTED_STATES = ["planned", "running", "completed", "failed", "canceled", "skipped", "interrupted", "abandoned", "unknown"] as const;
type ReportedState = typeof REPORTED_STATES[number];
export interface EvalLifecycle {
  initial_state: ReportedState; truncated: boolean;
  frontiers: { id: string; state: ReportedState; occurred_at: string; recorded_at: string }[];
}
export type EvalValue = JsonValue;
export interface EvalHeader {
  id: string; kind: EvalKind; title: string; occurred_at: string; recorded_at: string;
  repository?: string | null; model?: string | null;
}
export interface EvalRecord {
  id: string; title: string; occurred_at: string; recorded_at: string; project_id: string;
  provenance: { [key: string]: EvalValue };
  body: { [key: string]: EvalValue; kind: EvalKind };
  links: { relation: string; target_id: string }[];
  source?: { project_id: string; recorded_at: string; provenance: { [key: string]: EvalValue } } | null;
  lifecycle?: EvalLifecycle | null;
}
export interface EvalQuery { kind?: EvalKind; repository?: string; model?: string; search?: string; after?: string; limit?: number }
export interface EvalPage { records: EvalHeader[]; next_cursor: string | null }
export interface EvalHistory { records: EvalRecord[]; truncated: boolean }
export const EVAL_DETAIL_ROWS = 18;
export interface EvalsApi {
  list(project: string | null, query: EvalQuery, signal?: AbortSignal): Promise<EvalPage>;
  get(project: string | null, id: string, signal?: AbortSignal): Promise<EvalRecord | null>;
  why(project: string | null, id: string, signal?: AbortSignal): Promise<EvalHistory>;
}
function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function texts(value: Record<string, unknown>, fields: string[]): boolean {
  return fields.every(field => typeof value[field] === "string");
}
function kind(value: unknown): value is EvalKind {
  return typeof value === "string" && EVAL_KINDS.some(item => item === value);
}
function header(value: unknown): value is EvalHeader {
  return object(value) && texts(value, ["id", "title", "occurred_at", "recorded_at"]) && kind(value.kind) &&
    ["repository", "model"].every(field => value[field] === undefined || value[field] === null || typeof value[field] === "string");
}
function record(value: unknown): value is EvalRecord {
  return object(value) && texts(value, ["id", "title", "occurred_at", "recorded_at", "project_id"]) &&
    object(value.provenance) && isJsonValue(value.provenance) && object(value.body) && kind(value.body.kind) && isJsonValue(value.body) &&
    Array.isArray(value.links) && value.links.every(link => object(link) && texts(link, ["relation", "target_id"])) &&
    (value.source === undefined || value.source === null || source(value.source)) &&
    (value.lifecycle === undefined || value.lifecycle === null || lifecycle(value.lifecycle));
}
function source(value: unknown): boolean {
  return object(value) && texts(value, ["project_id", "recorded_at"]) && object(value.provenance) && isJsonValue(value.provenance);
}
function reportedState(value: unknown): value is ReportedState {
  return typeof value === "string" && REPORTED_STATES.some(state => state === value);
}
function lifecycle(value: unknown): value is EvalLifecycle {
  return object(value) && reportedState(value.initial_state) && typeof value.truncated === "boolean" &&
    Array.isArray(value.frontiers) && value.frontiers.length <= 64 && value.frontiers.every(frontier =>
      object(frontier) && texts(frontier, ["id", "occurred_at", "recorded_at"]) && reportedState(frontier.state));
}
function response(value: unknown, action: string): value is Record<string, unknown> {
  return object(value) && value.ok === true && value.action === action;
}
function invalid(): never { throw new Error("Nexus eval history returned an unexpected response"); }

export class NexusEvalsApi implements EvalsApi {
  constructor(private readonly client: Pick<NexusMcpClient, "call">, private readonly cwd: string) {}
  private call(project: string | null, fields: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.client.call("nexus_evals", { ...fields, ...(project === null ? {} : { project_id: project }) }, this.cwd, signal);
  }
  async list(project: string | null, query: EvalQuery, signal?: AbortSignal): Promise<EvalPage> {
    const value = await this.call(project, { action: "list", ...query }, signal);
    if (!response(value, "listed") || !Array.isArray(value.records) || !value.records.every(header) ||
      !(value.next_cursor === null || typeof value.next_cursor === "string")) return invalid();
    return { records: value.records, next_cursor: value.next_cursor };
  }
  async get(project: string | null, id: string, signal?: AbortSignal): Promise<EvalRecord | null> {
    const value = await this.call(project, { action: "get", id }, signal);
    if (!response(value, "detailed") || !(value.record === null || (record(value.record) && value.record.id === id))) return invalid();
    return value.record;
  }
  async why(project: string | null, id: string, signal?: AbortSignal): Promise<EvalHistory> {
    const value = await this.call(project, { action: "why", id }, signal);
    if (!response(value, "history") || !Array.isArray(value.records) || !value.records.every(record) || typeof value.truncated !== "boolean") return invalid();
    return { records: value.records, truncated: value.truncated };
  }
}
