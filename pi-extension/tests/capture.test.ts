// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { expect, it, vi } from "vitest";
import {
  registerAnalyticsCapture,
  type AnalyticsTransport,
} from "../src/capture.ts";

interface HandlerContext {
  cwd: string;
  model: { id: string };
  sessionManager: {
    getSessionId(): string;
    getLeafId(): string;
  };
}

it("records tool start and completion through the existing Nexus lifecycle calls", () => {
    const handlers = new Map<string, (event: never, context: HandlerContext) => void>();
    const pi = {
      on(name: string, handler: (event: never, context: HandlerContext) => void) {
        handlers.set(name, handler);
        return () => undefined;
      },
    } as unknown as ExtensionAPI;
    const transport: AnalyticsTransport = { call: vi.fn(), close: vi.fn() };
    registerAnalyticsCapture(pi, transport);
    const context = {
      cwd: "/repo",
      model: { id: "model-1" },
      sessionManager: {
        getSessionId: () => "session-1",
        getLeafId: () => "turn-1",
      },
    };

    handlers.get("tool_call")?.(
      {
        type: "tool_call",
        toolCallId: "tool-1",
        parentToolCallId: "parent-1",
        toolName: "bash",
        input: { command: "./scripts/check.sh --fast" },
      } as never,
      context,
    );
    handlers.get("tool_result")?.(
      {
        type: "tool_result",
        toolCallId: "tool-1",
        parentToolCallId: "parent-1",
        toolName: "bash",
        input: { command: "./scripts/check.sh --fast" },
        isError: false,
      } as never,
      context,
    );

    expect(transport.call).toHaveBeenNthCalledWith(
      1,
      "nexus_pre_tool_use",
      expect.objectContaining({
        session_id: "session-1",
        project_root: "/repo",
        agent: "pi",
        tool_use_id: "tool-1",
        parent_tool_use_id: "parent-1",
        tool_name: "bash",
        tool_input: { command: "./scripts/check.sh --fast" },
        turn_id: "turn-1",
        model: "model-1",
      }),
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      2,
      "nexus_post_tool_use",
      expect.objectContaining({ tool_use_id: "tool-1" }),
      "/repo",
    );
});

it("records explicit skills, failed tools, and closes at session shutdown", () => {
    const handlers = new Map<string, (event: never, context: HandlerContext) => void>();
    const pi = {
      on(name: string, handler: (event: never, context: HandlerContext) => void) {
        handlers.set(name, handler);
        return () => undefined;
      },
    } as unknown as ExtensionAPI;
    const transport: AnalyticsTransport = { call: vi.fn(), close: vi.fn() };
    registerAnalyticsCapture(pi, transport);
    const context = {
      cwd: "/repo",
      model: { id: "model-1" },
      sessionManager: {
        getSessionId: () => "session-1",
        getLeafId: () => "turn-1",
      },
    };

    handlers.get("input")?.(
      { type: "input", text: "/skill:TDD fix the bug", source: "interactive" } as never,
      context,
    );
    handlers.get("tool_result")?.(
      {
        type: "tool_result",
        toolCallId: "tool-2",
        toolName: "read",
        input: { path: "/missing" },
        isError: true,
      } as never,
      context,
    );
    handlers.get("session_shutdown")?.(
      { type: "session_shutdown", reason: "quit" } as never,
      context,
    );

    expect(transport.call).toHaveBeenNthCalledWith(
      1,
      "nexus_skill_use",
      expect.objectContaining({
        session_id: "session-1",
        skill_name: "tdd",
        evidence: "explicit_invocation",
        actor: "interactive",
      }),
      "/repo",
    );
    expect(transport.call).toHaveBeenNthCalledWith(
      2,
      "nexus_post_tool_failure",
      expect.objectContaining({ tool_use_id: "tool-2" }),
      "/repo",
    );
    expect(transport.close).toHaveBeenCalledOnce();
});

it("reports exposed and discoverable tool inventories before use and when the active set changes", () => {
  const handlers = new Map<string, (event: never, context: HandlerContext) => void>();
  let active = ["read"];
  const pi = {
    on(name: string, handler: (event: never, context: HandlerContext) => void) { handlers.set(name, handler); },
    getActiveTools: () => active,
    getAllTools: () => [
      { name: "read", description: "Read a file", exposure: "direct" },
      { name: "github.checks", description: "Inspect CI checks", exposure: "deferred" },
    ],
  } as unknown as ExtensionAPI;
  const transport: AnalyticsTransport = { call: vi.fn(), close: vi.fn() };
  registerAnalyticsCapture(pi, transport);
  const context = { cwd: "/repo", model: { id: "m1" }, sessionManager: { getSessionId: () => "session", getLeafId: () => "turn" } };
  handlers.get("session_start")?.({ type: "session_start" } as never, context);
  expect(transport.call).toHaveBeenCalledWith("nexus_analytics", expect.objectContaining({
    action: "inventory", scope: "host_tools", resources: expect.arrayContaining([
      expect.objectContaining({ name: "read", exposure: "exposed" }),
      expect.objectContaining({ name: "github.checks", exposure: "discoverable" }),
    ]),
  }), "/repo");
  active = ["read", "github.checks"];
  handlers.get("tool_call")?.({ type: "tool_call", toolCallId: "c1", toolName: "github.checks", input: {} } as never, context);
  expect(transport.call).toHaveBeenCalledWith("nexus_analytics", expect.objectContaining({
    resources: expect.arrayContaining([expect.objectContaining({ name: "github.checks", exposure: "exposed" })]),
  }), "/repo");
});

it("fingerprints tool definition changes and retries unchanged inventories after thirty seconds", async () => {
  const { inventoryCapture } = await import("../src/inventory-capture.ts");
  const clock = vi.spyOn(Date, "now").mockReturnValue(0);
  let parameters = { type: "string" };
  const pi = { getActiveTools: () => ["read"], getAllTools: () => [{ name: "read", description: "Read a file", parameters, exposure: "direct" }] } as unknown as ExtensionAPI;
  const context = { cwd: "/repo", sessionManager: { getSessionId: () => "session", getLeafId: () => "turn" } } as unknown as import("@earendil-works/pi-coding-agent").ExtensionContext;
  try {
    const capture = inventoryCapture(pi);
    const first = capture(context);
    expect(capture(context)).toBeUndefined();
    parameters = { type: "object" };
    const second = capture(context);
    expect(second).toBeDefined();
    expect((second?.resources as { version: string }[])[0].version).not.toBe((first?.resources as { version: string }[])[0].version);
    clock.mockReturnValue(30_001);
    expect(capture(context)).toBeDefined();
  } finally { clock.mockRestore(); }
});
