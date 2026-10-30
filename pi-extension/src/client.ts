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
import { decodeAnalytics, decodeCatalog, decodeExamples, type AnalyticsApi, type Catalog, type Examples, type ExampleFilter } from "./analytics-data.ts";
import { decodeUsage } from "./usage.ts";

function scopeArguments(projectId: string | null): Record<string, unknown> {
  return projectId === null ? {} : { project_id: projectId };
}

export async function fetchSnapshot(
  client: Pick<NexusMcpClient, "inspect">,
  cwd: string,
  projectId: string | null,
  signal?: AbortSignal,
): Promise<Snapshot> {
  const [dashboardJson, projectsJson] = await Promise.all([
    client.inspect("nexus_dashboard", scopeArguments(projectId), cwd, signal),
    client.inspect("nexus_projects", {}, cwd, signal),
  ]);
  return {
    dashboard: decodeDashboard(dashboardJson),
    projects: decodeProjects(projectsJson),
  };
}

export async function fetchUsage(
  client: Pick<NexusMcpClient, "inspect">,
  cwd: string,
  projectId: string | null,
  signal?: AbortSignal,
): Promise<UsageSummary> {
  return decodeUsage(
    await client.inspect("nexus_usage", scopeArguments(projectId), cwd, signal),
  );
}

export async function fetchAnalyticsUsage(client: Pick<NexusMcpClient, "inspect">, cwd: string, project: string | null, signal: AbortSignal): Promise<UsageSummary> {
  const usage = await fetchUsage(client, cwd, project, signal);
  try {
    const [summary, catalog, opportunities] = await Promise.all(["summary", "catalog", "opportunities"].map((action) => client.inspect("nexus_analytics", { action, ...scopeArguments(project) }, cwd, signal)));
    return { ...usage, analytics: decodeAnalytics({ summary, catalog, opportunities }) };
  } catch (error) {
    if (signal.aborted || !unsupportedAnalytics(error)) throw error;
    return { ...usage, analytics_error: "Activity analytics unavailable; showing legacy observations. Availability and semantic coverage are unknown." };
  }
}
export class NexusAnalyticsApi implements AnalyticsApi {
  constructor(private readonly client: Pick<NexusMcpClient, "inspect">, private readonly cwd: string) {}
  async catalog(project: string | null, offset: number, signal: AbortSignal): Promise<Catalog> {
    return decodeCatalog(await this.client.inspect("nexus_analytics", { action: "catalog", ...scopeArguments(project), offset }, this.cwd, signal));
  }
  async examples(project: string | null, filter: ExampleFilter, signal: AbortSignal): Promise<Examples> {
    return decodeExamples(await this.client.inspect("nexus_analytics", { action: "examples", ...scopeArguments(project), ...filter }, this.cwd, signal));
  }
}

function unsupportedAnalytics(error: unknown): boolean {
  return error instanceof Error && /unknown (tool|method)|unsupported.*analytics/iu.test(error.message);
}
