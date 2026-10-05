// @feature agent-memory
// @spec docs/features/agent-memory.md
// @entrypoint NexusMemoryApi
// @boundary dynamic-json
import type { NexusMcpClient } from "./mcp-client.ts";

export type MemoryScope =
  | { scope: "global" }
  | { scope: "project"; project_id: string };

export type MemoryReadScope =
  | MemoryScope
  | { scope: "layered"; project_id: string };

export interface MemoryProvenance {
  agent: string;
  session_id: string;
  model_id: string | null;
  config_hash: string;
}

export interface MemoryEntry {
  kind: "raw";
  id: string;
  scope: MemoryScope;
  ordinal: number;
  content: string;
  content_hash: string;
  provenance: MemoryProvenance;
  created_at: string;
}

export interface MemorySummary {
  kind: "summary";
  id: string;
  scope: MemoryScope;
  level: number;
  start_ordinal: number;
  end_ordinal: number;
  content: string;
  source_hash: string;
  provider: string;
  model_id: string | null;
  created_at: string;
}

export type MemoryNode = MemoryEntry | MemorySummary;

export interface MemoryOmission {
  scope: MemoryScope;
  start_ordinal: number;
  end_ordinal: number;
  reason: "missing_summary" | "budget" | "mixed";
}

export interface MemoryContextStatus {
  scope: MemoryReadScope;
  nodes: MemoryNode[];
  omitted: MemoryOmission[];
  item_count: number;
  byte_count: number;
}

export interface MemoryScopeHealth {
  scope: MemoryScope;
  raw_entries: number;
  summaries: number;
}

export type MemoryCompactionOutcome =
  | "succeeded"
  | "failed"
  | "timed_out"
  | "invalid_output"
  | "stale";

export interface MemoryProviderHealth {
  provider: string;
  model_id: string | null;
  last_outcome: MemoryCompactionOutcome;
  last_attempted_at: string;
  next_retry_at: string | null;
  consecutive_failures: number;
  cooling_down: boolean;
}

export interface MemoryHealth {
  scopes: MemoryScopeHealth[];
  providers: MemoryProviderHealth[];
  pending_summaries: number;
  failed_attempts: number;
  cooling_down_attempts: number;
  fallback_uses: number;
  degraded: boolean;
  last_activation_at: string | null;
}

export interface MemoryView {
  context: MemoryContextStatus;
  health: MemoryHealth;
}

export interface MemoryApi {
  load(): Promise<MemoryView>;
  expand(summaryId: string): Promise<MemoryNode[]>;
  search(regex: string): Promise<MemoryEntry[]>;
  add(scope: "global" | "project", content: string): Promise<void>;
  invalidate(summaryId: string): Promise<void>;
  consolidate(): Promise<void>;
}

export interface MemoryLifecycle {
  session_id: string;
  project_root: string;
  agent?: string;
  turn_id?: string;
  model?: string;
}

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function string(value: unknown, field: string): string {
  if (typeof value !== "string") throw new Error(`Nexus memory ${field} is invalid`);
  return value;
}

function nullableString(value: unknown, field: string): string | null {
  if (value === null) return null;
  return string(value, field);
}

function number(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`Nexus memory ${field} is invalid`);
  }
  return value;
}

function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") throw new Error(`Nexus memory ${field} is invalid`);
  return value;
}

function object(value: unknown, field: string): JsonObject {
  if (!isObject(value)) throw new Error(`Nexus memory ${field} is invalid`);
  return value;
}

function array(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`Nexus memory ${field} is invalid`);
  return value;
}

function response(value: unknown): JsonObject {
  const result = object(value, "response");
  if (result.ok !== true) {
    const detail = typeof result.error === "string" ? `: ${result.error}` : "";
    throw new Error(`Nexus memory request failed${detail}`);
  }
  return result;
}

function scope(value: unknown, field: string): MemoryScope {
  const candidate = object(value, field);
  if (candidate.scope === "global") return { scope: "global" };
  if (candidate.scope === "project") {
    return { scope: "project", project_id: string(candidate.project_id, `${field}.project_id`) };
  }
  throw new Error(`Nexus memory ${field} is invalid`);
}

function readScope(value: unknown): MemoryReadScope {
  const candidate = object(value, "context.scope");
  if (candidate.scope === "global") return { scope: "global" };
  if (candidate.scope === "project" || candidate.scope === "layered") {
    return {
      scope: candidate.scope,
      project_id: string(candidate.project_id, "context.scope.project_id"),
    };
  }
  throw new Error("Nexus memory context.scope is invalid");
}

function provenance(value: unknown): MemoryProvenance {
  const candidate = object(value, "entry.provenance");
  return {
    agent: string(candidate.agent, "entry.provenance.agent"),
    session_id: string(candidate.session_id, "entry.provenance.session_id"),
    model_id: nullableString(candidate.model_id, "entry.provenance.model_id"),
    config_hash: string(candidate.config_hash, "entry.provenance.config_hash"),
  };
}

function rawNode(value: JsonObject): MemoryEntry {
  return {
    kind: "raw",
    id: string(value.id, "entry.id"),
    scope: scope(value.scope, "entry.scope"),
    ordinal: number(value.ordinal, "entry.ordinal"),
    content: string(value.content, "entry.content"),
    content_hash: string(value.content_hash, "entry.content_hash"),
    provenance: provenance(value.provenance),
    created_at: string(value.created_at, "entry.created_at"),
  };
}

function summaryNode(value: JsonObject): MemorySummary {
  return {
    kind: "summary",
    id: string(value.id, "summary.id"),
    scope: scope(value.scope, "summary.scope"),
    level: number(value.level, "summary.level"),
    start_ordinal: number(value.start_ordinal, "summary.start_ordinal"),
    end_ordinal: number(value.end_ordinal, "summary.end_ordinal"),
    content: string(value.content, "summary.content"),
    source_hash: string(value.source_hash, "summary.source_hash"),
    provider: string(value.provider, "summary.provider"),
    model_id: nullableString(value.model_id, "summary.model_id"),
    created_at: string(value.created_at, "summary.created_at"),
  };
}

function node(value: unknown): MemoryNode {
  const candidate = object(value, "node");
  if (candidate.kind === "raw") return rawNode(candidate);
  if (candidate.kind === "summary") return summaryNode(candidate);
  throw new Error("Nexus memory node.kind is invalid");
}

function entry(value: unknown): MemoryEntry {
  const candidate = object(value, "entry");
  if (candidate.kind !== undefined && candidate.kind !== "raw") {
    throw new Error("Nexus memory entry.kind is invalid");
  }
  return rawNode(candidate);
}

function context(value: unknown): MemoryContextStatus {
  const candidate = object(value, "context");
  return {
    scope: readScope(candidate.scope),
    nodes: array(candidate.nodes, "context.nodes").map(node),
    omitted: array(candidate.omitted, "context.omitted").map((item) => {
      const omission = object(item, "context.omitted item");
      const reason = omission.reason;
      if (reason !== "missing_summary" && reason !== "budget" && reason !== "mixed") {
        throw new Error("Nexus memory omission.reason is invalid");
      }
      return {
        scope: scope(omission.scope, "omission.scope"),
        start_ordinal: number(omission.start_ordinal, "omission.start_ordinal"),
        end_ordinal: number(omission.end_ordinal, "omission.end_ordinal"),
        reason,
      };
    }),
    item_count: number(candidate.item_count, "context.item_count"),
    byte_count: number(candidate.byte_count, "context.byte_count"),
  };
}

function health(value: unknown): MemoryHealth {
  const candidate = object(value, "health");
  return {
    scopes: array(candidate.scopes, "health.scopes").map((item) => {
      const scopeHealth = object(item, "health scope");
      return {
        scope: scope(scopeHealth.scope, "health.scope"),
        raw_entries: number(scopeHealth.raw_entries, "health.raw_entries"),
        summaries: number(scopeHealth.summaries, "health.summaries"),
      };
    }),
    providers: array(candidate.providers, "health.providers").map((item) => {
      const providerHealth = object(item, "provider health");
      const lastOutcome = providerHealth.last_outcome;
      if (
        lastOutcome !== "succeeded" &&
        lastOutcome !== "failed" &&
        lastOutcome !== "timed_out" &&
        lastOutcome !== "invalid_output" &&
        lastOutcome !== "stale"
      ) {
        throw new Error("Nexus memory provider.last_outcome is invalid");
      }
      return {
        provider: string(providerHealth.provider, "provider.provider"),
        model_id: nullableString(providerHealth.model_id, "provider.model_id"),
        last_outcome: lastOutcome,
        last_attempted_at: string(
          providerHealth.last_attempted_at,
          "provider.last_attempted_at",
        ),
        next_retry_at: nullableString(providerHealth.next_retry_at, "provider.next_retry_at"),
        consecutive_failures: number(
          providerHealth.consecutive_failures,
          "provider.consecutive_failures",
        ),
        cooling_down: boolean(providerHealth.cooling_down, "provider.cooling_down"),
      };
    }),
    pending_summaries: number(candidate.pending_summaries, "health.pending_summaries"),
    failed_attempts: number(candidate.failed_attempts, "health.failed_attempts"),
    cooling_down_attempts: number(
      candidate.cooling_down_attempts,
      "health.cooling_down_attempts",
    ),
    fallback_uses: number(candidate.fallback_uses, "health.fallback_uses"),
    degraded: boolean(candidate.degraded, "health.degraded"),
    last_activation_at: nullableString(
      candidate.last_activation_at,
      "health.last_activation_at",
    ),
  };
}

export class NexusMemoryApi implements MemoryApi {
  constructor(
    private readonly client: Pick<NexusMcpClient, "call">,
    private readonly cwd: string,
    private readonly lifecycle: () => MemoryLifecycle,
  ) {}

  async load(): Promise<MemoryView> {
    const [statusValue, contextValue] = await Promise.all([
      this.client.call("nexus_memory_status", { project_root: this.cwd }, this.cwd),
      this.client.call(
        "nexus_memory_context",
        { scope: "layered", project_root: this.cwd },
        this.cwd,
      ),
    ]);
    const statusResponse = response(statusValue);
    const contextResponse = response(contextValue);
    return {
      health: health(statusResponse.health),
      context: context(contextResponse.context),
    };
  }

  async expand(summaryId: string): Promise<MemoryNode[]> {
    const result = response(await this.client.call(
      "nexus_memory_expand",
      { summary_id: summaryId },
      this.cwd,
    ));
    return array(result.nodes, "nodes").map(node);
  }

  async search(regex: string): Promise<MemoryEntry[]> {
    const result = response(await this.client.call(
      "nexus_memory_search",
      { scope: "layered", project_root: this.cwd, regex, limit: 50 },
      this.cwd,
    ));
    return array(result.memories, "memories").map(entry);
  }

  async add(scopeValue: "global" | "project", content: string): Promise<void> {
    const result = response(await this.client.call(
      "nexus_memory_add",
      { ...this.lifecycle(), scope: scopeValue, content },
      this.cwd,
    ));
    boolean(result.inserted, "inserted");
    entry(result.entry);
  }

  async invalidate(summaryId: string): Promise<void> {
    const result = response(await this.client.call(
      "nexus_memory_invalidate",
      { summary_id: summaryId },
      this.cwd,
    ));
    number(result.invalidated_summaries, "invalidated_summaries");
  }

  async consolidate(): Promise<void> {
    const result = response(await this.client.call(
      "nexus_memory_consolidate",
      {},
      this.cwd,
    ));
    for (const field of ["attempted", "succeeded", "failed", "stale", "fallback_uses"] as const) {
      number(result[field], field);
    }
  }
}
