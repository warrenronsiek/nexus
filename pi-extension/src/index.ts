// @feature observability-ui
// @spec docs/features/observability-ui.md
// @entrypoint pi-extension
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { fetchSnapshot, startNexus } from "./client.ts";
import { NexusDashboardComponent } from "./component.ts";

export default function nexusExtension(pi: ExtensionAPI): void {
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
        await ctx.ui.custom<void>((tui, theme, _keybindings, done) => {
          return new NexusDashboardComponent(
            tui,
            theme,
            snapshot,
            (projectId, signal) => fetchSnapshot(baseUrl, projectId, signal),
            () => done(),
          );
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.ui.notify(`Nexus is unavailable: ${message}`, "error");
      }
    },
  });
}
