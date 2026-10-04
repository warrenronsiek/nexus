// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json

export interface Counts {
  active_sessions: number;
  active_claims: number;
  open_conflicts: number;
  events: number;
}

export interface RecordWindow<T> {
  items: T[];
  truncated: boolean;
}

export interface EventRecord {
  id: number;
  project_id: string;
  session_id: string | null;
  kind: string;
  payload: unknown;
  created_at: string;
}

export interface SessionRecord {
  session_id: string;
  project_id: string;
  agent: string;
  worktree: string | null;
  status: "active" | "stopped";
  task_summary: string | null;
  last_seen_at: string;
}

export interface ClaimRecord {
  id: string;
  project_id: string;
  session_id: string;
  tool_use_id: string;
  path: string;
  operation: "write" | "delete" | "rename";
  line_start: number | null;
  line_end: number | null;
  state: "claimed" | "modified" | "released";
  expires_at: string;
  updated_at: string;
}

export interface ConflictRecord {
  id: string;
  project_id: string;
  left_claim_id: string;
  right_claim_id: string;
  path: string;
  severity: "info" | "warning" | "critical";
  kind: string;
  status: "open" | "resolved";
  message: string;
  created_at: string;
  updated_at: string;
}

export interface ProjectSummary {
  project_id: string;
  worktrees: string[];
  agents: string[];
  last_seen_at: string;
}

export interface Dashboard {
  ok: true;
  project_id: string | null;
  generated_at: string;
  refresh_interval_ms: number;
  counts: Counts;
  events: RecordWindow<EventRecord>;
  sessions: RecordWindow<SessionRecord>;
  claims: RecordWindow<ClaimRecord>;
  conflicts: RecordWindow<ConflictRecord>;
}

export interface Snapshot {
  dashboard: Dashboard;
  projects: ProjectSummary[];
}

export type UsageKind = "tool" | "script" | "skill";

export interface UsageItem {
  kind: UsageKind;
  name: string;
  count: number;
  sessions: number;
  succeeded?: number;
  failed?: number;
  observed?: number;
  evidence?: string | Record<string, number>;
}

export interface UsageSummary {
  ok: true;
  project_id: string | null;
  window_started_at: string;
  window_ended_at: string;
  tools: UsageItem[];
  skills: UsageItem[];
  capture_health: Record<string, unknown>;
}

type JsonObject = Record<string, unknown>;
type FieldGuard = (value: unknown) => boolean;
type FieldShape = Record<string, FieldGuard>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isNullableString(value: unknown): value is string | null {
  return value === null || isString(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || isNumber(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(isString);
}

function isAnyValue(_value: unknown): boolean {
  return true;
}

function oneOf(values: readonly string[]): FieldGuard {
  return (value) => typeof value === "string" && values.includes(value);
}

function hasShape(value: unknown, shape: FieldShape): value is JsonObject {
  return (
    isObject(value) &&
    Object.entries(shape).every(([field, guard]) => field in value && guard(value[field]))
  );
}

function isWindow(value: unknown, itemGuard: (item: unknown) => boolean): boolean {
  return (
    isObject(value) &&
    typeof value.truncated === "boolean" &&
    Array.isArray(value.items) &&
    value.items.every(itemGuard)
  );
}

const eventShape: FieldShape = {
  id: isNumber,
  project_id: isString,
  session_id: isNullableString,
  kind: isString,
  payload: isAnyValue,
  created_at: isString,
};
const sessionShape: FieldShape = {
  session_id: isString,
  project_id: isString,
  agent: isString,
  worktree: isNullableString,
  status: oneOf(["active", "stopped"]),
  task_summary: isNullableString,
  last_seen_at: isString,
};
const claimShape: FieldShape = {
  id: isString,
  project_id: isString,
  session_id: isString,
  tool_use_id: isString,
  path: isString,
  operation: oneOf(["write", "delete", "rename"]),
  line_start: isNullableNumber,
  line_end: isNullableNumber,
  state: oneOf(["claimed", "modified", "released"]),
  expires_at: isString,
  updated_at: isString,
};
const conflictShape: FieldShape = {
  id: isString,
  project_id: isString,
  left_claim_id: isString,
  right_claim_id: isString,
  path: isString,
  severity: oneOf(["info", "warning", "critical"]),
  kind: isString,
  status: oneOf(["open", "resolved"]),
  message: isString,
  created_at: isString,
  updated_at: isString,
};
const projectShape: FieldShape = {
  project_id: isString,
  worktrees: isStringArray,
  agents: isStringArray,
  last_seen_at: isString,
};
const countsShape: FieldShape = {
  active_sessions: isNumber,
  active_claims: isNumber,
  open_conflicts: isNumber,
  events: isNumber,
};

function isEvent(value: unknown): value is EventRecord {
  return hasShape(value, eventShape);
}

function isSession(value: unknown): value is SessionRecord {
  return hasShape(value, sessionShape);
}

function isClaim(value: unknown): value is ClaimRecord {
  return hasShape(value, claimShape);
}

function isConflict(value: unknown): value is ConflictRecord {
  return hasShape(value, conflictShape);
}

function isProject(value: unknown): value is ProjectSummary {
  return hasShape(value, projectShape);
}

function isCounts(value: unknown): value is Counts {
  return hasShape(value, countsShape);
}

export function decodeDashboard(value: unknown): Dashboard {
  if (
    !isObject(value) ||
    value.ok !== true ||
    !isNullableString(value.project_id) ||
    !isString(value.generated_at) ||
    !isNumber(value.refresh_interval_ms) ||
    value.refresh_interval_ms <= 0 ||
    !isCounts(value.counts) ||
    !isWindow(value.events, isEvent) ||
    !isWindow(value.sessions, isSession) ||
    !isWindow(value.claims, isClaim) ||
    !isWindow(value.conflicts, isConflict)
  ) {
    throw new Error("Nexus dashboard returned an unexpected response");
  }
  return value as unknown as Dashboard;
}

export function decodeProjects(value: unknown): ProjectSummary[] {
  if (!isObject(value) || value.ok !== true || !Array.isArray(value.projects) || !value.projects.every(isProject)) {
    throw new Error("Nexus projects returned an unexpected response");
  }
  return value.projects;
}
