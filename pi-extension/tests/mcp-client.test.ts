// @feature agent-memory
// @spec docs/features/agent-memory.md
import { expect, it, vi } from "vitest";
import {
  NexusMcpClient,
  type McpProcess,
} from "../src/mcp-client.ts";

type HarnessStdin = {
  write: ReturnType<typeof vi.fn>;
  once: ReturnType<typeof vi.fn>;
  end: ReturnType<typeof vi.fn>;
};

interface HarnessListeners {
  stdoutData?: (chunk: unknown) => void;
  drain?: () => void;
  childError?: (error?: unknown) => void;
  childExit: Array<() => void>;
}

function stdinHarness(listeners: HarnessListeners): HarnessStdin {
  return {
    write: vi.fn((_value: string) => true),
    once: vi.fn((event: string, listener: () => void) => {
      if (event === "drain") listeners.drain = listener;
    }),
    end: vi.fn(),
  };
}

function childHarness(
  listeners: HarnessListeners,
  stdin: HarnessStdin,
  kill: ReturnType<typeof vi.fn>,
): McpProcess {
  return {
    stdin,
    stdout: {
      on: vi.fn((event: string, listener: (chunk: unknown) => void) => {
        if (event === "data") listeners.stdoutData = listener;
      }),
    },
    once: vi.fn((event: string, listener: (error?: unknown) => void) => {
      if (event === "error") listeners.childError = listener;
      if (event === "exit") listeners.childExit.push(listener);
    }),
    kill,
  } as unknown as McpProcess;
}

function processHarness(): {
  child: McpProcess;
  emitDrain(): void;
  emitError(error: Error): void;
  emitExit(): void;
  emitStdout(value: string): void;
  stdin: HarnessStdin;
  kill: ReturnType<typeof vi.fn>;
} {
  const listeners: HarnessListeners = { childExit: [] };
  const stdin = stdinHarness(listeners);
  const kill = vi.fn(() => true);
  const child = childHarness(listeners, stdin, kill);
  return {
    child,
    emitDrain() {
      const listener = listeners.drain;
      listeners.drain = undefined;
      listener?.();
    },
    emitError(error: Error) {
      listeners.childError?.(error);
    },
    emitExit() {
      for (const listener of listeners.childExit.splice(0)) listener();
    },
    emitStdout(value: string) {
      listeners.stdoutData?.(Buffer.from(value));
    },
    stdin,
    kill,
  };
}

it("terminates a stalled MCP child when the client closes", async () => {
  vi.useFakeTimers();
  try {
    const harness = processHarness();
    const client = new NexusMcpClient(() => harness.child);
    const pending = client.call("nexus_dashboard", {}, "/repo");
    const rejected = expect(pending).rejects.toThrow("client closed");

    client.close();
    await rejected;

    expect(harness.stdin.end).toHaveBeenCalledOnce();
    expect(harness.kill).toHaveBeenCalledWith("SIGTERM");
    await vi.advanceTimersByTimeAsync(250);
    expect(harness.kill).toHaveBeenLastCalledWith("SIGKILL");
  } finally {
    vi.useRealTimers();
  }
});

it("does not force-kill an MCP child that exits during the grace period", async () => {
  vi.useFakeTimers();
  try {
    const harness = processHarness();
    const client = new NexusMcpClient(() => harness.child);
    const pending = client.call("nexus_dashboard", {}, "/repo");
    const rejected = expect(pending).rejects.toThrow("client closed");

    client.close();
    harness.emitExit();
    await rejected;
    await vi.advanceTimersByTimeAsync(250);

    expect(harness.kill).toHaveBeenCalledTimes(1);
    expect(harness.kill).toHaveBeenCalledWith("SIGTERM");
  } finally {
    vi.useRealTimers();
  }
});

it("correlates out-of-order MCP responses while reusing one child", async () => {
  const harness = processHarness();
  const spawnMcp = vi.fn(() => harness.child);
  const client = new NexusMcpClient(spawnMcp);

  const context = client.call("nexus_memory_context", { project_root: "/repo" }, "/repo");
  const status = client.call("nexus_memory_status", {}, "/repo");

  expect(spawnMcp).toHaveBeenCalledOnce();
  expect(harness.stdin.write).toHaveBeenCalledTimes(2);
  const requests = harness.stdin.write.mock.calls.map(([line]) => JSON.parse(String(line)));
  expect(requests.map((request) => request.params.name)).toEqual([
    "nexus_memory_context",
    "nexus_memory_status",
  ]);

  harness.emitStdout(
    `${JSON.stringify({
      jsonrpc: "2.0",
      id: requests[1].id,
      result: { structuredContent: { ok: true, pending: 2 } },
    })}\n${JSON.stringify({
      jsonrpc: "2.0",
      id: requests[0].id,
      result: { structuredContent: { ok: true, nodes: ["recent"] } },
    })}\n`,
  );

  await expect(status).resolves.toEqual({ ok: true, pending: 2 });
  await expect(context).resolves.toEqual({ ok: true, nodes: ["recent"] });
  client.close();
  expect(harness.stdin.end).toHaveBeenCalledOnce();
});

it("buffers a response split across stdout chunks", async () => {
  const harness = processHarness();
  const client = new NexusMcpClient(() => harness.child);
  const response = client.call("nexus_memory_status", {}, "/repo");
  const request = JSON.parse(String(harness.stdin.write.mock.calls[0][0]));

  const line = JSON.stringify({
    jsonrpc: "2.0",
    id: request.id,
    result: { structuredContent: { ok: true } },
  });
  harness.emitStdout(line.slice(0, 17));
  harness.emitStdout(`${line.slice(17)}\n`);

  await expect(response).resolves.toEqual({ ok: true });
  client.close();
});

it("does not send a queued mutation after that request has timed out", async () => {
  vi.useFakeTimers();
  try {
    const harness = processHarness();
    harness.stdin.write.mockReturnValueOnce(false);
    const client = new NexusMcpClient(() => harness.child, 256, 10);
    const blocking = client.call("nexus_memory_status", {}, "/repo");
    const blockingResult = blocking.catch((error: unknown) => error);
    const mutation = client.call(
      "nexus_memory_invalidate",
      { summary_id: "summary-1" },
      "/repo",
    );
    const mutationResult = expect(mutation).rejects.toThrow("timed out after 10ms");

    await vi.advanceTimersByTimeAsync(11);
    await mutationResult;
    harness.emitDrain();

    expect(harness.stdin.write).toHaveBeenCalledTimes(1);
    client.close();
    expect(await blockingResult).toBeInstanceOf(Error);
  } finally {
    vi.useRealTimers();
  }
});

it("does not send a queued read after its polling signal is aborted", async () => {
  const harness = processHarness();
  harness.stdin.write.mockReturnValueOnce(false);
  const client = new NexusMcpClient(() => harness.child);
  const blocking = client.call("nexus_memory_status", {}, "/repo");
  const blockingResult = blocking.catch((error: unknown) => error);
  const controller = new AbortController();
  const dashboard = client.call(
    "nexus_dashboard",
    {},
    "/repo",
    controller.signal,
  );
  const dashboardResult = expect(dashboard).rejects.toThrow("scope changed");

  controller.abort(new Error("scope changed"));
  await dashboardResult;
  harness.emitDrain();

  expect(harness.stdin.write).toHaveBeenCalledTimes(1);
  client.close();
  expect(await blockingResult).toBeInstanceOf(Error);
});

it("rejects excess queued calls when the backpressure queue is full", async () => {
  const harness = processHarness();
  harness.stdin.write.mockReturnValueOnce(false);
  const client = new NexusMcpClient(() => harness.child, 1);
  const blocking = client.call("nexus_memory_status", {}, "/repo");
  const queued = client.call("nexus_memory_context", {}, "/repo");
  const excess = client.call("nexus_memory_search", { regex: "typed" }, "/repo");
  const blockingResult = blocking.catch((error: unknown) => error);
  const queuedResult = queued.catch((error: unknown) => error);

  await expect(excess).rejects.toThrow("request queue is full");
  expect(harness.stdin.write).toHaveBeenCalledTimes(1);
  client.close();
  expect(await blockingResult).toBeInstanceOf(Error);
  expect(await queuedResult).toBeInstanceOf(Error);
});

it("rejects every pending call when the MCP child errors", async () => {
  const harness = processHarness();
  const client = new NexusMcpClient(() => harness.child);
  const first = client.call("nexus_memory_status", {}, "/repo");
  const second = client.call("nexus_memory_context", {}, "/repo");
  const firstResult = expect(first).rejects.toThrow("broken pipe");
  const secondResult = expect(second).rejects.toThrow("broken pipe");

  harness.emitError(new Error("broken pipe"));

  await firstResult;
  await secondResult;
});
