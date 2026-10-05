// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface LifecycleArguments {
  session_id: string;
  project_root: string;
  agent: string;
  turn_id: string | undefined;
  model: string | undefined;
}

export function lifecycleArguments(context: ExtensionContext): LifecycleArguments {
  return {
    session_id: context.sessionManager.getSessionId(),
    project_root: context.cwd,
    agent: "pi",
    turn_id: context.sessionManager.getLeafId() ?? undefined,
    model: context.model?.id,
  };
}
