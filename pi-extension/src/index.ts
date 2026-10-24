// @feature work-history
// @spec docs/features/work-history.md
// @feature evals
// @spec docs/features/evals.md
// @feature papercuts
// @spec docs/features/papercuts.md
// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @feature sos
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/sos.md
// @entrypoint pi-extension
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAnalyticsCapture } from "./capture.ts";
import { fetchSnapshot, fetchUsage, fetchAnalyticsUsage, NexusAnalyticsApi } from "./client.ts";
import { NexusDashboardComponent } from "./component.ts";
import { sanitizeDisplayText } from "./display.ts";
import { NexusEvalsApi } from "./evals.ts";
import { NexusWorkApi } from "./work.ts";
import { NexusMcpClient } from "./mcp-client.ts";
import { registerMemoryContext } from "./memory-context.ts";
import { MemoryPrompts } from "./memory-prompts.ts";
import { NexusMemoryApi } from "./memory.ts";
import { NexusPapercutsApi } from "./papercuts.ts";
import { NexusSosApi } from "./sos.ts";

export default function nexusExtension(pi: ExtensionAPI): void {
  const client = new NexusMcpClient();
  registerAnalyticsCapture(pi, client);
  registerMemoryContext(pi, client);
  pi.registerCommand("nexus", {
    description: "Open the live Nexus coordination dashboard",
    handler: async (_args, ctx) => {
      if (ctx.mode !== "tui") {
        ctx.ui.notify("The Nexus dashboard requires Pi's interactive terminal mode", "error");
        return;
      }
      try {
        ctx.ui.notify("Connecting to Nexus…", "info");
        const snapshot = await fetchSnapshot(client, ctx.cwd, null);
        const memoryApi = new NexusMemoryApi(client, ctx.cwd);
        const sosApi = new NexusSosApi(client, ctx.cwd);
        await ctx.ui.custom<void>((tui, theme, _keybindings, done) => {
          return new NexusDashboardComponent(
            tui,
            theme,
            snapshot,
            {
              loadSnapshot: (projectId, signal) =>
                fetchSnapshot(client, ctx.cwd, projectId, signal),
              loadUsage: (projectId, signal) =>
                fetchUsage(client, ctx.cwd, projectId, signal),
              loadAnalyticsUsage: (projectId, signal) => fetchAnalyticsUsage(client, ctx.cwd, projectId, signal),
              analyticsApi: new NexusAnalyticsApi(client, ctx.cwd),
              papercutsApi: new NexusPapercutsApi(client, ctx.cwd),
              evalsApi: new NexusEvalsApi(client, ctx.cwd),
              workApi: new NexusWorkApi(client, ctx.cwd),
              sosApi,
              memoryApi,
              memoryPrompts: new MemoryPrompts(ctx.ui),
              done: () => done(),
            },
          );
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(sanitizeDisplayText(`Nexus is unavailable: ${message}`), "error");
      }
    },
  });
}
