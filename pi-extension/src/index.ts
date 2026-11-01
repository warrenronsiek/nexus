// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @entrypoint pi-extension
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerAnalyticsCapture } from "./capture.ts";
import { NexusMcpClient } from "./mcp-client.ts";
import { registerMemoryContext } from "./memory-context.ts";

export default function nexusExtension(pi: ExtensionAPI): void {
  const client = new NexusMcpClient();
  registerAnalyticsCapture(pi, client);
  registerMemoryContext(pi, client);
}
