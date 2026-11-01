// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import nexusExtension from "../src/index.ts";

describe("Pi extension entry point", () => {
  it("captures host events and activates memory without registering a terminal interface", () => {
    const events: string[] = [];
    const registerCommand = vi.fn();
    const pi = {
      on(name: string) { events.push(name); },
      registerCommand,
    } as unknown as ExtensionAPI;

    nexusExtension(pi);

    expect(events).toEqual(expect.arrayContaining([
      "session_start", "tool_call", "tool_result", "input", "session_shutdown", "before_agent_start",
    ]));
    expect(registerCommand).not.toHaveBeenCalled();
  });
});
