// @feature papercuts
// @feature observability-ui
// @spec docs/features/papercuts.md
// @spec docs/features/observability-ui.md
// @boundary dynamic-json: validate the MCP list projection before displaying external data.
import type { NexusMcpClient } from "./mcp-client.ts";

export interface Papercut {
  id: string;
  project_id: string;
  title: string;
  description: string;
  status: "open" | "resolved" | "consolidated";
  upvote_count: number;
  last_upvoted_at: string | null;
  created_at: string;
  updated_at: string;
  resolution: string | null;
  consolidated_into: string | null;
}
export interface PapercutsList { papercuts: Papercut[]; truncated: boolean }
export interface PapercutCursor { updated_at: string; id: string }
export interface PapercutsApi {
  list(projectId: string | null, signal?: AbortSignal, after?: PapercutCursor): Promise<PapercutsList>;
}
function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function isPapercut(value: unknown): value is Papercut {
  return object(value) &&
    ["id", "project_id", "title", "description", "created_at", "updated_at"].every(key => typeof value[key] === "string") &&
    ["last_upvoted_at", "resolution", "consolidated_into"].every(key => value[key] === null || typeof value[key] === "string") &&
    ["open", "resolved", "consolidated"].includes(String(value.status)) &&
    typeof value.upvote_count === "number" && Number.isSafeInteger(value.upvote_count) && value.upvote_count >= 0;
}
export class NexusPapercutsApi implements PapercutsApi {
  constructor(private readonly client: Pick<NexusMcpClient, "inspect">, private readonly cwd: string) {}
  async list(projectId: string | null, signal?: AbortSignal, after?: PapercutCursor): Promise<PapercutsList> {
    const value = await this.client.inspect("nexus_papercuts", {
      action: "list", ...(projectId === null ? {} : { project_id: projectId }),
      ...(after === undefined ? {} : { after }),
    }, this.cwd, signal);
    if (!object(value) || value.ok !== true || value.action !== "listed" ||
      !Array.isArray(value.papercuts) || !value.papercuts.every(isPapercut) || typeof value.truncated !== "boolean") {
      throw new Error("Nexus papercuts list returned an unexpected response");
    }
    return { papercuts: value.papercuts, truncated: value.truncated };
  }
}
