// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @entrypoint fetchSnapshot
// @boundary dynamic-json
import {
  decodeDashboard,
  decodeProjects,
  type Snapshot,
  type UsageSummary,
} from "./domain.ts";
import type { NexusMcpClient } from "./mcp-client.ts";
import { decodeUsage } from "./usage.ts";

function scopeArguments(projectId: string | null): Record<string, unknown> {
  return projectId === null ? {} : { project_id: projectId };
}

export async function fetchSnapshot(
  client: Pick<NexusMcpClient, "call">,
  cwd: string,
  projectId: string | null,
  signal?: AbortSignal,
): Promise<Snapshot> {
  const [dashboardJson, projectsJson] = await Promise.all([
    client.call("nexus_dashboard", scopeArguments(projectId), cwd, signal),
    client.call("nexus_projects", {}, cwd, signal),
  ]);
  return {
    dashboard: decodeDashboard(dashboardJson),
    projects: decodeProjects(projectsJson),
  };
}

export async function fetchUsage(
  client: Pick<NexusMcpClient, "call">,
  cwd: string,
  projectId: string | null,
  signal?: AbortSignal,
): Promise<UsageSummary> {
  return decodeUsage(
    await client.call("nexus_usage", scopeArguments(projectId), cwd, signal),
  );
}
