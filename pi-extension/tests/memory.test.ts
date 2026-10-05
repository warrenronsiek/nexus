// @feature agent-memory
// @spec docs/features/agent-memory.md
import { expect, it, vi } from "vitest";
import { NexusMemoryApi } from "../src/memory.ts";

const projectScope = { scope: "project", project_id: "project-1" } as const;
const rawEntry = {
  id: "raw-1",
  scope: projectScope,
  ordinal: 4,
  content: "Prefer typed interfaces.",
  content_hash: "raw-hash",
  provenance: {
    agent: "pi",
    session_id: "session-1",
    model_id: null,
    config_hash: "config-1",
  },
  created_at: "2026-10-04T12:00:00Z",
};
const summaryNode = {
  kind: "summary",
  id: "summary-1",
  scope: projectScope,
  level: 1,
  start_ordinal: 0,
  end_ordinal: 4,
  content: "Use narrow typed boundaries.",
  source_hash: "summary-hash",
  provider: "codex",
  model_id: "gpt-test",
  created_at: "2026-10-04T11:00:00Z",
};

function client(responses: Record<string, unknown>) {
  return {
    call: vi.fn(async (name: string) => {
      if (!(name in responses)) throw new Error(`unexpected ${name}`);
      return responses[name];
    }),
  };
}

  it("loads typed context and health over the shared local MCP transport", async () => {
    const transport = client({
      nexus_memory_status: {
        ok: true,
        health: {
          scopes: [{ scope: projectScope, raw_entries: 1, summaries: 1, future: true }],
          providers: [{
            provider: "codex",
            model_id: "gpt-test",
            last_outcome: "succeeded",
            last_attempted_at: "2026-10-04T12:04:00Z",
            next_retry_at: null,
            consecutive_failures: 0,
            cooling_down: false,
            future: true,
          }],
          pending_summaries: 0,
          failed_attempts: 0,
          cooling_down_attempts: 0,
          fallback_uses: 0,
          degraded: false,
          last_activation_at: null,
          future_provider_field: "ignored",
        },
      },
      nexus_memory_context: {
        ok: true,
        context: {
          scope: { scope: "layered", project_id: "project-1" },
          nodes: [summaryNode, { kind: "raw", ...rawEntry }],
          omitted: [{
            scope: projectScope,
            start_ordinal: 0,
            end_ordinal: 3,
            reason: "mixed",
          }],
          item_count: 2,
          byte_count: 64,
          future_context_field: "ignored",
        },
      },
    });
    const api = new NexusMemoryApi(transport, "/repo", () => ({
      session_id: "session-1",
      project_root: "/repo",
    }));

    const view = await api.load();

    expect(view.context.nodes).toHaveLength(2);
    expect(view.context.omitted).toEqual([{
      scope: projectScope,
      start_ordinal: 0,
      end_ordinal: 3,
      reason: "mixed",
    }]);
    expect(view.health.degraded).toBe(false);
    expect(view.health.providers).toEqual([{
      provider: "codex",
      model_id: "gpt-test",
      last_outcome: "succeeded",
      last_attempted_at: "2026-10-04T12:04:00Z",
      next_retry_at: null,
      consecutive_failures: 0,
      cooling_down: false,
    }]);
    expect(transport.call).toHaveBeenNthCalledWith(
      1,
      "nexus_memory_status",
      { project_root: "/repo" },
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      2,
      "nexus_memory_context",
      { scope: "layered", project_root: "/repo" },
      "/repo",
    );
  });

  it("normalizes raw entries and maps every pane action to its locked MCP method", async () => {
    const transport = client({
      nexus_memory_expand: { ok: true, nodes: [{ kind: "raw", ...rawEntry }] },
      nexus_memory_search: { ok: true, memories: [rawEntry] },
      nexus_memory_add: { ok: true, inserted: true, entry: rawEntry },
      nexus_memory_invalidate: { ok: true, invalidated_summaries: 1 },
      nexus_memory_consolidate: {
        ok: true,
        attempted: 2,
        succeeded: 1,
        failed: 0,
        stale: 1,
        fallback_uses: 0,
      },
    });
    const api = new NexusMemoryApi(transport, "/repo", () => ({
      session_id: "session-1",
      project_root: "/repo",
      agent: "pi",
      turn_id: "turn-2",
      model: "model-1",
    }));

    expect(await api.expand("summary-1")).toEqual([{ kind: "raw", ...rawEntry }]);
    expect(await api.search("typed.*interfaces")).toEqual([{ kind: "raw", ...rawEntry }]);
    await api.add("project", "Prefer typed interfaces.");
    await api.invalidate("summary-1");
    await api.consolidate();

    expect(transport.call).toHaveBeenNthCalledWith(
      2,
      "nexus_memory_search",
      { scope: "layered", project_root: "/repo", regex: "typed.*interfaces", limit: 50 },
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      3,
      "nexus_memory_add",
      {
        session_id: "session-1",
        project_root: "/repo",
        agent: "pi",
        turn_id: "turn-2",
        model: "model-1",
        scope: "project",
        content: "Prefer typed interfaces.",
      },
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      4,
      "nexus_memory_invalidate",
      { summary_id: "summary-1" },
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      5,
      "nexus_memory_consolidate",
      {},
      "/repo",
    );
  });

  it("rejects malformed external responses rather than leaking dynamic JSON", async () => {
    const transport = client({
      nexus_memory_status: {
        ok: true,
        health: {
          scopes: [],
          providers: [],
          pending_summaries: 0,
          failed_attempts: 0,
          cooling_down_attempts: 0,
          fallback_uses: 0,
          degraded: "no",
          last_activation_at: null,
        },
      },
      nexus_memory_context: {
        ok: true,
        context: {
          scope: { scope: "global" },
          nodes: [],
          omitted: [],
          item_count: 0,
          byte_count: 0,
        },
      },
    });
    const api = new NexusMemoryApi(transport, "/repo", () => ({
      session_id: "session-1",
      project_root: "/repo",
    }));

    await expect(api.load()).rejects.toThrow("health.degraded is invalid");
  });
