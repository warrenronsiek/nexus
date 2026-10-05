// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @entrypoint pi-extension
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAnalyticsCapture } from "./capture.ts";
import { fetchSnapshot, fetchUsage, startNexus } from "./client.ts";
import { NexusDashboardComponent } from "./component.ts";
import { lifecycleArguments } from "./host-context.ts";
import { NexusMcpClient } from "./mcp-client.ts";
import { registerMemoryContext } from "./memory-context.ts";
import { MemoryPrompts } from "./memory-prompts.ts";
import { NexusMemoryApi } from "./memory.ts";

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
        const baseUrl = await startNexus(ctx.cwd);
        const snapshot = await fetchSnapshot(baseUrl, null, AbortSignal.timeout(5000));
        const memoryApi = new NexusMemoryApi(
          client,
          ctx.cwd,
          () => lifecycleArguments(ctx),
        );
        await ctx.ui.custom<void>((tui, theme, _keybindings, done) => {
          return new NexusDashboardComponent(
            tui,
            theme,
            snapshot,
            {
              loadSnapshot: (projectId, signal) => fetchSnapshot(baseUrl, projectId, signal),
              loadUsage: (projectId, signal) => fetchUsage(baseUrl, projectId, signal),
              memoryApi,
              memoryPrompts: new MemoryPrompts(ctx.ui),
              done: () => done(),
            },
          );
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(`Nexus is unavailable: ${message}`, "error");
      }
    },
  });
}
