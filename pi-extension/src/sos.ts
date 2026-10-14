// @feature sos
// @feature observability-ui
// @spec docs/features/sos.md
// @spec docs/features/observability-ui.md
// @boundary dynamic-json
import type { NexusMcpClient } from "./mcp-client.ts";

export type SosIssueStatus = "open" | "resolved" | "deprecated";

export interface SosIssue {
  id: string;
  project_id: string;
  title: string;
  summary: string;
  status: SosIssueStatus;
  resolution: string | null;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  first_reported_at: string;
  last_reported_at: string;
  importance_count: number;
}

export interface SosActor {
  agent: string;
  session_id: string;
  model_id: string | null;
  config_hash: string;
}

export interface SosReport {
  id: string;
  project_id: string;
  issue_id: string | null;
  message: string;
  type: string;
  environment: string;
  version: string;
  model: string;
  trace: string;
  metadata: Record<string, unknown>;
  reporter: SosActor;
  created_at: string;
}

export interface SosTriageAttempt {
  id: string;
  project_id: string;
  report_id: string;
  issue_id: string | null;
  outcome: "created" | "linked" | "failed";
  provider: string;
  model_id: string | null;
  rationale: string;
  generated_title: string | null;
  generated_summary: string | null;
  config_hash: string;
  created_at: string;
}

export interface SosLifecycleEvent {
  id: string;
  project_id: string;
  issue_id: string;
  transition: "resolved" | "deprecated" | "reopened";
  reason: string;
  actor: SosActor;
  created_at: string;
}

export interface SosIssueDetail {
  issue: SosIssue;
  reports: SosReport[];
  reports_truncated: boolean;
  triage_attempts: SosTriageAttempt[];
  triage_attempts_truncated: boolean;
  lifecycle_events: SosLifecycleEvent[];
  lifecycle_events_truncated: boolean;
}

export interface SosList {
  ok: true;
  issues: SosIssue[];
  untriaged_report_count: number;
  truncated: boolean;
}

export interface SosApi {
  list(signal?: AbortSignal): Promise<SosList>;
  get(issueId: string, signal?: AbortSignal): Promise<SosIssueDetail>;
}

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isNullableString(value: unknown): value is string | null {
  return value === null || isString(value);
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasTextFields(
  value: unknown,
  required: readonly string[],
  nullable: readonly string[] = [],
): value is JsonObject {
  return isObject(value) &&
    required.every((field) => isString(value[field])) &&
    nullable.every((field) => isNullableString(value[field]));
}

function isActor(value: unknown): value is SosActor {
  return isObject(value) &&
    isString(value.agent) &&
    isString(value.session_id) &&
    isNullableString(value.model_id) &&
    isString(value.config_hash);
}

function isIssue(value: unknown): value is SosIssue {
  return hasTextFields(
    value,
    [
      "id",
      "project_id",
      "title",
      "summary",
      "created_at",
      "updated_at",
      "first_reported_at",
      "last_reported_at",
    ],
    ["resolution", "closed_at"],
  ) &&
    ["open", "resolved", "deprecated"].includes(String(value.status)) &&
    isCount(value.importance_count);
}

function isReport(value: unknown): value is SosReport {
  return isObject(value) &&
    isString(value.id) &&
    isString(value.project_id) &&
    isNullableString(value.issue_id) &&
    isString(value.message) &&
    isString(value.type) &&
    isString(value.environment) &&
    isString(value.version) &&
    isString(value.model) &&
    isString(value.trace) &&
    isObject(value.metadata) &&
    isActor(value.reporter) &&
    isString(value.created_at);
}

function isTriageAttempt(value: unknown): value is SosTriageAttempt {
  return hasTextFields(
    value,
    ["id", "project_id", "report_id", "provider", "rationale", "config_hash", "created_at"],
    ["issue_id", "model_id", "generated_title", "generated_summary"],
  ) &&
    ["created", "linked", "failed"].includes(String(value.outcome));
}

function isLifecycleEvent(value: unknown): value is SosLifecycleEvent {
  return isObject(value) &&
    isString(value.id) &&
    isString(value.project_id) &&
    isString(value.issue_id) &&
    ["resolved", "deprecated", "reopened"].includes(String(value.transition)) &&
    isString(value.reason) &&
    isActor(value.actor) &&
    isString(value.created_at);
}

function isDetail(value: unknown): value is SosIssueDetail {
  return isObject(value) &&
    isIssue(value.issue) &&
    Array.isArray(value.reports) && value.reports.every(isReport) &&
    typeof value.reports_truncated === "boolean" &&
    Array.isArray(value.triage_attempts) && value.triage_attempts.every(isTriageAttempt) &&
    typeof value.triage_attempts_truncated === "boolean" &&
    Array.isArray(value.lifecycle_events) && value.lifecycle_events.every(isLifecycleEvent) &&
    typeof value.lifecycle_events_truncated === "boolean";
}

export function decodeSosList(value: unknown): SosList {
  if (!isObject(value) || value.ok !== true || !Array.isArray(value.issues) ||
    !value.issues.every(isIssue) || !isCount(value.untriaged_report_count) ||
    typeof value.truncated !== "boolean") {
    throw new Error("Nexus SOS list returned an unexpected response");
  }
  return value as unknown as SosList;
}

export function decodeSosDetail(value: unknown): SosIssueDetail {
  if (!isObject(value) || value.ok !== true || !isDetail(value.detail)) {
    throw new Error("Nexus SOS detail returned an unexpected response");
  }
  return value.detail;
}

export class NexusSosApi implements SosApi {
  constructor(
    private readonly client: Pick<NexusMcpClient, "call">,
    private readonly cwd: string,
  ) {}

  async list(signal?: AbortSignal): Promise<SosList> {
    return decodeSosList(
      await this.client.call("nexus_sos", { action: "list" }, this.cwd, signal),
    );
  }

  async get(issueId: string, signal?: AbortSignal): Promise<SosIssueDetail> {
    return decodeSosDetail(
      await this.client.call(
        "nexus_sos",
        { action: "get", issue_id: issueId },
        this.cwd,
        signal,
      ),
    );
  }
}
