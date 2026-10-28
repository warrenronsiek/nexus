// @feature observability-ui
// @spec docs/features/observability-ui.md
// Shared host composition keeps the native and Pi dashboards on the same APIs.
import type { DashboardOptions } from "./component.ts";
import { fetchSnapshot, fetchUsage, fetchAnalyticsUsage, NexusAnalyticsApi } from "./client.ts";
import { NexusConcernsApi } from "./concerns.ts";
import { NexusEvalsApi } from "./evals.ts";
import { NexusWorkApi } from "./work.ts";
import { NexusMemoryApi } from "./memory.ts";
import { NexusPapercutsApi } from "./papercuts.ts";
import { NexusSosApi } from "./sos.ts";
import type { MemoryInteractionPrompts } from "./memory-pane.ts";
import type { NexusMcpClient } from "./mcp-client.ts";

export function dashboardOptions(client: NexusMcpClient, cwd: string, memoryPrompts: MemoryInteractionPrompts, done: () => void): DashboardOptions {
  return {
    loadSnapshot: (project, signal) => fetchSnapshot(client, cwd, project, signal),
    loadUsage: (project, signal) => fetchUsage(client, cwd, project, signal),
    loadAnalyticsUsage: (project, signal) => fetchAnalyticsUsage(client, cwd, project, signal),
    analyticsApi: new NexusAnalyticsApi(client, cwd), papercutsApi: new NexusPapercutsApi(client, cwd),
    evalsApi: new NexusEvalsApi(client, cwd), workApi: new NexusWorkApi(client, cwd), concernsApi: new NexusConcernsApi(client, cwd),
    sosApi: new NexusSosApi(client, cwd), memoryApi: new NexusMemoryApi(client, cwd), memoryPrompts, done,
  };
}
