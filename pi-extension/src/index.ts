// @feature work-history
// @spec docs/features/work-history.md
// @feature code-concerns
// @spec docs/features/code-concerns.md
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
import { dashboardOptions } from "./dashboard-options.ts";
import { NexusDashboardConnection } from "./connection.ts";
import { sanitizeDisplayText } from "./display.ts";
import { NexusMcpClient } from "./mcp-client.ts";
import { registerMemoryContext } from "./memory-context.ts";
import { MemoryPrompts } from "./memory-prompts.ts";

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
        await ctx.ui.custom<void>((tui, theme, _keybindings, done) => {
          return new NexusDashboardConnection(tui, theme,
            dashboardOptions(client, ctx.cwd, new MemoryPrompts(ctx.ui), () => done()));
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(sanitizeDisplayText(`Nexus is unavailable: ${message}`), "error");
      }
    },
  });
}
