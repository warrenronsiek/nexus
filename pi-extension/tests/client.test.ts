// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { describe, expect, it, vi } from "vitest";
import { fetchSnapshot, fetchUsage } from "../src/client.ts";
import type { NexusMcpClient } from "../src/mcp-client.ts";

function queryClient(responses: Record<string, unknown>): {
  call: ReturnType<typeof vi.fn>;
  client: Pick<NexusMcpClient, "call">;
} {
  const call = vi.fn(async (toolName: string) => responses[toolName]);
  return { call, client: { call } };
}

describe("Pi local Nexus reads", () => {
  it("loads coordination and project snapshots through the existing MCP child", async () => {
    const dashboard = {
      ok: true,
      project_id: "project-1",
      generated_at: "2026-10-05T12:00:00Z",
      refresh_interval_ms: 2_000,
      counts: { events: 0, active_sessions: 0, active_claims: 0, open_conflicts: 0 },
      events: { items: [], truncated: false },
      sessions: { items: [], truncated: false },
      claims: { items: [], truncated: false },
      conflicts: { items: [], truncated: false },
    };
    const projects = { ok: true, projects: [] };
    const { call, client } = queryClient({
      nexus_dashboard: dashboard,
      nexus_projects: projects,
    });

    await expect(fetchSnapshot(client, "/repo", "project-1")).resolves.toEqual({
      dashboard,
      projects: [],
    });
    expect(call).toHaveBeenCalledWith(
      "nexus_dashboard",
      { project_id: "project-1" },
      "/repo",
      undefined,
    );
    expect(call).toHaveBeenCalledWith(
      "nexus_projects",
      {},
      "/repo",
      undefined,
    );
  });

  it("loads usage through MCP with the selected project scope", async () => {
    const response = {
      ok: true,
      project_id: "project-1",
      window_started_at: "2026-09-28T12:00:00Z",
      window_ended_at: "2026-10-05T12:00:00Z",
      tools: [],
      skills: [],
      capture_health: {},
    };
    const { call, client } = queryClient({ nexus_usage: response });

    await expect(fetchUsage(client, "/repo", "project-1")).resolves.toEqual(response);
    expect(call).toHaveBeenCalledWith(
      "nexus_usage",
      { project_id: "project-1" },
      "/repo",
      undefined,
    );
  });
});
