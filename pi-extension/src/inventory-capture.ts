// @feature usage-analytics
// @spec docs/features/usage-analytics.md
// @boundary host-tool-inventory-json
import { createHash } from "node:crypto";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { lifecycleArguments } from "./host-context.ts";

export function inventoryCapture(pi: ExtensionAPI): (context: ExtensionContext) => Record<string, unknown> | undefined {
  let previous = "";
  let reportedAt = -Infinity;
  return (context) => {
    if (typeof pi.getAllTools !== "function" || typeof pi.getActiveTools !== "function") return;
    const active = new Set(pi.getActiveTools());
    const tools = pi.getAllTools();
    const resources = tools.slice(0, 256).map((tool) => ({
      name: tool.name.slice(0, 512), kind: "tool", origin: "host_tool",
      exposure: active.has(tool.name) ? "exposed" : tool.exposure === "hidden" ? "declared" : "discoverable",
      description: tool.description.slice(0, 512).replace(/[\u0000-\u001f\u007f-\u009f]/gu, " "),
      activities: [],
      version: createHash("sha256").update(JSON.stringify([tool.name, tool.description, tool.parameters, tool.promptGuidelines])).digest("hex"),
    }));
    const signature = JSON.stringify([context.sessionManager.getSessionId(), resources]);
    if (signature === previous && Date.now() - reportedAt < 30_000) return;
    previous = signature;
    reportedAt = Date.now();
    return { action: "inventory", ...lifecycleArguments(context), scope: "host_tools", resources, complete: tools.length <= 256 };
  };
}
