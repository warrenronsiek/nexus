// @feature runtime
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @entrypoint NexusMcpClient
// @boundary dynamic-json
import { spawn } from "node:child_process";

interface McpStdin {
  write(value: string): boolean;
  once(event: "drain", listener: () => void): unknown;
  end(): void;
}

interface McpStdout {
  on(event: "data", listener: (chunk: unknown) => void): unknown;
}

export interface McpProcess {
  stdin: McpStdin;
  stdout: McpStdout;
  once(event: "exit" | "error", listener: (error?: unknown) => void): unknown;
}

export type McpSpawner = (cwd: string) => McpProcess;

interface PendingCall {
  reject(error: Error): void;
  resolve(value: unknown): void;
  timeout: NodeJS.Timeout;
}

interface QueuedWrite {
  id: number;
  line: string;
}

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function spawnMcp(cwd: string): McpProcess {
  const child = spawn("nexus", ["mcp"], {
    cwd,
    stdio: ["pipe", "pipe", "ignore"],
  });
  if (child.stdin === null || child.stdout === null) {
    throw new Error("could not open Nexus MCP streams");
  }
  return child as unknown as McpProcess;
}

export class NexusMcpClient {
  private child: McpProcess | undefined;
  private cwd: string | undefined;
  private requestId = 0;
  private stdoutBuffer = "";
  private backpressured = false;
  private queued: QueuedWrite[] = [];
  private readonly calls = new Map<number, PendingCall>();

  constructor(
    private readonly start: McpSpawner = spawnMcp,
    private readonly maximumPending = 256,
  ) {}

  call(
    toolName: string,
    argumentsValue: Record<string, unknown>,
    cwd: string,
    timeoutMs = 5_000,
  ): Promise<unknown> {
    const child = this.ensureChild(cwd);
    if (child === undefined) return Promise.reject(new Error("Nexus MCP is unavailable"));
    const id = ++this.requestId;
    const line = `${JSON.stringify({
      jsonrpc: "2.0",
      id,
      method: "tools/call",
      params: { name: toolName, arguments: argumentsValue },
    })}\n`;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.reject(id, new Error(`Nexus MCP request timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      timeout.unref();
      this.calls.set(id, { resolve, reject, timeout });
      this.enqueue(child, { id, line });
    });
  }

  close(): void {
    const child = this.child;
    this.reset(new Error("Nexus MCP client closed"));
    if (child !== undefined) {
      try {
        child.stdin.end();
      } catch {
        // Nexus is advisory and must never disrupt Pi shutdown.
      }
    }
  }

  private ensureChild(cwd: string): McpProcess | undefined {
    if (this.child !== undefined && this.cwd === cwd) return this.child;
    if (this.child !== undefined) this.close();
    try {
      const child = this.start(cwd);
      this.child = child;
      this.cwd = cwd;
      this.stdoutBuffer = "";
      child.stdout.on("data", (chunk) => this.receive(child, chunk));
      child.once("exit", () => this.release(child, new Error("Nexus MCP exited")));
      child.once("error", (error) => {
        const cause = error instanceof Error ? error : new Error("Nexus MCP failed");
        this.release(child, cause);
      });
      return child;
    } catch {
      return undefined;
    }
  }

  private enqueue(child: McpProcess, write: QueuedWrite): void {
    if (this.backpressured) {
      if (this.queued.length < this.maximumPending) {
        this.queued.push(write);
      } else {
        this.reject(write.id, new Error("Nexus MCP request queue is full"));
      }
      return;
    }
    this.write(child, write);
  }

  private write(child: McpProcess, write: QueuedWrite): void {
    try {
      if (child.stdin.write(write.line)) return;
      this.backpressured = true;
      child.stdin.once("drain", () => this.flush(child));
    } catch (error) {
      const cause = error instanceof Error ? error : new Error("Nexus MCP write failed");
      this.release(child, cause);
    }
  }

  private flush(child: McpProcess): void {
    if (this.child !== child) return;
    this.backpressured = false;
    while (!this.backpressured) {
      const write = this.queued.shift();
      if (write === undefined) return;
      this.write(child, write);
    }
  }

  private receive(child: McpProcess, chunk: unknown): void {
    if (this.child !== child) return;
    this.stdoutBuffer += Buffer.isBuffer(chunk) ? chunk.toString("utf8") : String(chunk);
    while (true) {
      const boundary = this.stdoutBuffer.indexOf("\n");
      if (boundary < 0) return;
      const line = this.stdoutBuffer.slice(0, boundary).trim();
      this.stdoutBuffer = this.stdoutBuffer.slice(boundary + 1);
      if (line.length > 0) this.receiveLine(child, line);
    }
  }

  private receiveLine(child: McpProcess, line: string): void {
    let response: unknown;
    try {
      response = JSON.parse(line);
    } catch {
      this.release(child, new Error("Nexus MCP returned malformed JSON"));
      return;
    }
    if (!isObject(response) || typeof response.id !== "number") return;
    const call = this.calls.get(response.id);
    if (call === undefined) return;
    this.calls.delete(response.id);
    clearTimeout(call.timeout);
    if (isObject(response.error)) {
      const message = typeof response.error.message === "string"
        ? response.error.message
        : "Nexus MCP request failed";
      call.reject(new Error(message));
      return;
    }
    const result = response.result;
    call.resolve(isObject(result) && "structuredContent" in result
      ? result.structuredContent
      : result);
  }

  private reject(id: number, error: Error): void {
    const call = this.calls.get(id);
    if (call === undefined) return;
    this.calls.delete(id);
    this.queued = this.queued.filter((write) => write.id !== id);
    clearTimeout(call.timeout);
    call.reject(error);
  }

  private release(child: McpProcess, error: Error): void {
    if (this.child !== child) return;
    this.reset(error);
  }

  private reset(error: Error): void {
    this.child = undefined;
    this.cwd = undefined;
    this.stdoutBuffer = "";
    this.backpressured = false;
    this.queued = [];
    for (const [id] of this.calls) this.reject(id, error);
  }
}
