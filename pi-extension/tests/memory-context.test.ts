// @feature agent-memory
// @spec docs/features/agent-memory.md
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import {
  registerMemoryContext,
  type MemoryContextClient,
} from "../src/memory-context.ts";

interface HandlerContext {
  cwd: string;
  model: { id: string };
  sessionManager: {
    getSessionId(): string;
    getLeafId(): string;
  };
}

function beforeStartHarness(client: MemoryContextClient): {
  handler: (event: never, context: HandlerContext) => Promise<unknown>;
  context: HandlerContext;
} {
  let handler: ((event: never, context: HandlerContext) => Promise<unknown>) | undefined;
  const pi = {
    on(name: string, candidate: typeof handler) {
      if (name === "before_agent_start") handler = candidate;
      return () => undefined;
    },
  } as unknown as ExtensionAPI;
  registerMemoryContext(pi, client);
  if (handler === undefined) throw new Error("before_agent_start was not registered");
  return {
    handler,
    context: {
      cwd: "/repo",
      model: { id: "model-1" },
      sessionManager: {
        getSessionId: () => "session-1",
        getLeafId: () => "turn-1",
      },
    },
  };
}

describe("Pi memory activation", () => {
  it("injects returned startup memory as hidden untrusted context", async () => {
    const client: MemoryContextClient = {
      call: vi.fn(async () => ({
        hookSpecificOutput: {
          additionalContext: "#0 project uses release trains",
        },
      })),
    };
    const { handler, context } = beforeStartHarness(client);

    const result = await handler(
      { type: "before_agent_start", prompt: "Continue the release work" } as never,
      context,
    );

    expect(client.call).toHaveBeenCalledWith(
      "nexus_user_prompt",
      {
        session_id: "session-1",
        project_root: "/repo",
        agent: "pi",
        turn_id: "turn-1",
        model: "model-1",
        prompt: "Continue the release work",
      },
      "/repo",
    );
    expect(result).toEqual({
      message: {
        customType: "nexus-memory-context",
        content: expect.stringMatching(/untrusted historical context[\s\S]*release trains/iu),
        display: false,
      },
    });
  });

  it("fails open when Nexus is unavailable or returns no memory", async () => {
    const unavailable = beforeStartHarness({
      call: vi.fn(async () => Promise.reject(new Error("offline"))),
    });
    await expect(
      unavailable.handler(
        { type: "before_agent_start", prompt: "hello" } as never,
        unavailable.context,
      ),
    ).resolves.toBeUndefined();

    const empty = beforeStartHarness({
      call: vi.fn(async () => ({ ok: true })),
    });
    await expect(
      empty.handler(
        { type: "before_agent_start", prompt: "hello" } as never,
        empty.context,
      ),
    ).resolves.toBeUndefined();
  });
});
