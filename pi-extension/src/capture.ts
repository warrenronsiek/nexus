// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
import { spawn } from "node:child_process";
import type {
  ExtensionAPI,
  ExtensionContext,
  InputEvent,
  ToolCallEvent,
  ToolResultEvent,
} from "@earendil-works/pi-coding-agent";

export interface AnalyticsTransport {
  send(toolName: string, argumentsValue: Record<string, unknown>, cwd: string): void | Promise<void>;
  close(): void;
}

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
  once(event: "exit" | "error", listener: () => void): unknown;
}

export type McpSpawner = (cwd: string) => McpProcess;

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

export class McpAnalyticsClient implements AnalyticsTransport {
  private child: McpProcess | undefined;
  private cwd: string | undefined;
  private requestId = 0;
  private backpressured = false;
  private pending: string[] = [];

  constructor(
    private readonly start: McpSpawner = spawnMcp,
    private readonly maximumPending = 256,
  ) {}

  send(toolName: string, argumentsValue: Record<string, unknown>, cwd: string): void {
    const child = this.ensureChild(cwd);
    if (child === undefined) return;
    const line = `${JSON.stringify({
      jsonrpc: "2.0",
      id: ++this.requestId,
      method: "tools/call",
      params: { name: toolName, arguments: argumentsValue },
    })}\n`;
    if (this.backpressured) {
      if (this.pending.length < this.maximumPending) this.pending.push(line);
      return;
    }
    this.write(child, line);
  }

  close(): void {
    const child = this.child;
    this.child = undefined;
    this.cwd = undefined;
    this.backpressured = false;
    this.pending = [];
    if (child !== undefined) {
      try {
        child.stdin.end();
      } catch {
        // Capture is advisory and must never disrupt Pi shutdown.
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
      child.stdout.on("data", () => undefined);
      child.once("exit", () => this.release(child));
      child.once("error", () => this.release(child));
      return child;
    } catch {
      return undefined;
    }
  }

  private write(child: McpProcess, line: string): void {
    try {
      if (child.stdin.write(line)) return;
      this.backpressured = true;
      child.stdin.once("drain", () => this.flush(child));
    } catch {
      this.release(child);
    }
  }

  private flush(child: McpProcess): void {
    if (this.child !== child) return;
    this.backpressured = false;
    while (!this.backpressured) {
      const line = this.pending.shift();
      if (line === undefined) return;
      this.write(child, line);
    }
  }

  private release(child: McpProcess): void {
    if (this.child !== child) return;
    this.child = undefined;
    this.cwd = undefined;
    this.backpressured = false;
    this.pending = [];
  }
}

function modelId(context: ExtensionContext): string | undefined {
  return context.model?.id;
}

function turnId(context: ExtensionContext): string | undefined {
  return context.sessionManager.getLeafId() ?? undefined;
}

function commonArguments(context: ExtensionContext): Record<string, unknown> {
  return {
    session_id: context.sessionManager.getSessionId(),
    project_root: context.cwd,
    agent: "pi",
    turn_id: turnId(context),
    model: modelId(context),
  };
}

function sendBestEffort(
  transport: AnalyticsTransport,
  toolName: string,
  argumentsValue: Record<string, unknown>,
  cwd: string,
): void {
  try {
    void Promise.resolve(transport.send(toolName, argumentsValue, cwd)).catch(() => undefined);
  } catch {
    // Analytics is advisory; host execution must continue unchanged.
  }
}

function toolArguments(
  event: ToolCallEvent | ToolResultEvent,
  context: ExtensionContext,
): Record<string, unknown> {
  return {
    ...commonArguments(context),
    tool_use_id: event.toolCallId,
    parent_tool_use_id: event.parentToolCallId,
    tool_name: event.toolName,
    tool_input: event.input,
  };
}

function explicitSkill(input: InputEvent): string | undefined {
  return /^\s*\/skill:([a-z0-9][a-z0-9_-]*)\b/iu.exec(input.text)?.[1]?.toLowerCase();
}

export function registerAnalyticsCapture(
  pi: ExtensionAPI,
  transport: AnalyticsTransport = new McpAnalyticsClient(),
): void {
  let skillSequence = 0;
  pi.on("tool_call", (event, context) => {
    sendBestEffort(
      transport,
      "nexus_pre_tool_use",
      toolArguments(event, context),
      context.cwd,
    );
  });
  pi.on("tool_result", (event, context) => {
    sendBestEffort(
      transport,
      event.isError ? "nexus_post_tool_failure" : "nexus_post_tool_use",
      toolArguments(event, context),
      context.cwd,
    );
  });
  pi.on("input", (event, context) => {
    const skillName = explicitSkill(event);
    if (skillName === undefined) return;
    sendBestEffort(
      transport,
      "nexus_skill_use",
      {
        ...commonArguments(context),
        skill_name: skillName,
        invocation_id: `${context.sessionManager.getSessionId()}:skill:${++skillSequence}`,
        evidence: "explicit_invocation",
        actor: event.source,
      },
      context.cwd,
    );
  });
  pi.on("session_shutdown", () => transport.close());
}
