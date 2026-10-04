// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import nexusExtension from "../src/index.ts";

describe("Pi extension entry point", () => {
  it("registers /nexus and explains when terminal UI is unavailable", async () => {
    let command: { handler: (args: string, context: unknown) => Promise<void> } | undefined;
    const pi = {
      on() {
        return () => undefined;
      },
      registerCommand(name: string, options: typeof command) {
        expect(name).toBe("nexus");
        command = options;
      },
    } as unknown as ExtensionAPI;
    nexusExtension(pi);

    const notify = vi.fn();
    await command?.handler("", { mode: "print", ui: { notify } });
    expect(notify).toHaveBeenCalledWith("The Nexus dashboard requires Pi's interactive terminal mode", "error");
  });
});
