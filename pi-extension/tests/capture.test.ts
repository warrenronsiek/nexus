// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { expect, it, vi } from "vitest";
import {
  McpAnalyticsClient,
  registerAnalyticsCapture,
  type AnalyticsTransport,
  type McpProcess,
} from "../src/capture.ts";

interface HandlerContext {
  cwd: string;
  model: { id: string };
  sessionManager: {
    getSessionId(): string;
    getLeafId(): string;
  };
}

it("reuses one MCP child for multiple observations", () => {
    const stdin = {
      write: vi.fn((_value: string) => true),
      once: vi.fn(),
      end: vi.fn(),
    };
    const child = {
      stdin,
      stdout: { on: vi.fn() },
      once: vi.fn(),
    } as unknown as McpProcess;
    const spawnMcp = vi.fn(() => child);
    const client = new McpAnalyticsClient(spawnMcp);

    client.send("nexus_pre_tool_use", { tool_use_id: "one" }, "/repo");
    client.send("nexus_post_tool_use", { tool_use_id: "one" }, "/repo");

    expect(spawnMcp).toHaveBeenCalledOnce();
    expect(stdin.write).toHaveBeenCalledTimes(2);
    expect(String(stdin.write.mock.calls[0][0])).toContain('"name":"nexus_pre_tool_use"');
    expect(String(stdin.write.mock.calls[1][0])).toContain('"name":"nexus_post_tool_use"');
    client.close();
    expect(stdin.end).toHaveBeenCalledOnce();
});

it("records tool start and completion through the existing Nexus lifecycle calls", () => {
    const handlers = new Map<string, (event: never, context: HandlerContext) => void>();
    const pi = {
      on(name: string, handler: (event: never, context: HandlerContext) => void) {
        handlers.set(name, handler);
        return () => undefined;
      },
    } as unknown as ExtensionAPI;
    const transport: AnalyticsTransport = { send: vi.fn(), close: vi.fn() };
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

    expect(transport.send).toHaveBeenNthCalledWith(
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
    expect(transport.send).toHaveBeenNthCalledWith(
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
    const transport: AnalyticsTransport = { send: vi.fn(), close: vi.fn() };
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

    expect(transport.send).toHaveBeenNthCalledWith(
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
    expect(transport.send).toHaveBeenNthCalledWith(
      2,
      "nexus_post_tool_failure",
      expect.objectContaining({ tool_use_id: "tool-2" }),
      "/repo",
    );
    expect(transport.close).toHaveBeenCalledOnce();
});
