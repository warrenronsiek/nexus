// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
import type {
  ExtensionAPI,
  ExtensionContext,
  InputEvent,
  ToolCallEvent,
  ToolResultEvent,
} from "@earendil-works/pi-coding-agent";
import { NexusMcpClient } from "./mcp-client.ts";
import { lifecycleArguments } from "./host-context.ts";

export type AnalyticsTransport = Pick<NexusMcpClient, "call" | "close">;

function sendBestEffort(
  transport: AnalyticsTransport,
  toolName: string,
  argumentsValue: Record<string, unknown>,
  cwd: string,
): void {
  try {
    void Promise.resolve(transport.call(toolName, argumentsValue, cwd)).catch(() => undefined);
  } catch {
    // Analytics is advisory; host execution must continue unchanged.
  }
}

function toolArguments(
  event: ToolCallEvent | ToolResultEvent,
  context: ExtensionContext,
): Record<string, unknown> {
  return {
    ...lifecycleArguments(context),
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
  transport: AnalyticsTransport = new NexusMcpClient(),
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
        ...lifecycleArguments(context),
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
