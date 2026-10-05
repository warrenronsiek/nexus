// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
import type { Theme } from "@earendil-works/pi-coding-agent";
import type { TUI } from "@earendil-works/pi-tui";
import { expect, it, vi } from "vitest";
import {
  NexusDashboardComponent,
  nextDashboardTab,
} from "../src/component.ts";
import type { Snapshot, UsageSummary } from "../src/domain.ts";
import type { MemoryInteractionPrompts } from "../src/memory-pane.ts";
import type { MemoryApi, MemoryView } from "../src/memory.ts";

const snapshot = {
  dashboard: {
    ok: true,
    project_id: null,
    generated_at: "2026-10-04T12:00:00Z",
    refresh_interval_ms: 2000,
    counts: {
      active_sessions: 1,
      active_claims: 0,
      open_conflicts: 0,
      events: 1,
    },
    events: { items: [], truncated: false },
    sessions: { items: [], truncated: false },
    claims: { items: [], truncated: false },
    conflicts: { items: [], truncated: false },
  },
  projects: [],
} satisfies Snapshot;

const usage = {
  ok: true,
  project_id: null,
  window_started_at: "2026-09-27T12:00:00Z",
  window_ended_at: "2026-10-04T12:00:00Z",
  tools: [{ kind: "tool", name: "bash", count: 7, sessions: 2 }],
  skills: [{ kind: "skill", name: "tdd", count: 3, sessions: 2 }],
  capture_health: {},
} satisfies UsageSummary;

const theme = {
  fg: (_color: string, text: string) => text,
  bold: (text: string) => text,
} as Theme;

const memoryView: MemoryView = {
  context: {
    scope: { scope: "layered", project_id: "project-1" },
    nodes: [{
      kind: "summary",
      id: "summary-1",
      scope: { scope: "project", project_id: "project-1" },
      level: 1,
      start_ordinal: 0,
      end_ordinal: 2,
      content: "Keep boundaries typed.",
      source_hash: "summary-hash",
      provider: "codex",
      model_id: "model-1",
      created_at: "2026-10-04T11:00:00Z",
    }],
    omitted: [],
    item_count: 1,
    byte_count: 22,
  },
  health: {
    scopes: [{
      scope: { scope: "project", project_id: "project-1" },
      raw_entries: 2,
      summaries: 1,
    }],
    providers: [],
    pending_summaries: 0,
    failed_attempts: 0,
    cooling_down_attempts: 0,
    fallback_uses: 0,
    degraded: false,
    last_activation_at: null,
  },
};

function memoryHarness(): { api: MemoryApi; load: ReturnType<typeof vi.fn>; prompts: MemoryInteractionPrompts } {
  const load = vi.fn(async () => memoryView);
  return {
    load,
    api: {
      load,
      expand: vi.fn(async () => []),
      search: vi.fn(async () => []),
      add: vi.fn(async () => undefined),
      invalidate: vi.fn(async () => undefined),
      consolidate: vi.fn(async () => undefined),
    },
    prompts: {
      searchRegex: vi.fn(async () => undefined),
      newMemory: vi.fn(async () => undefined),
      confirmInvalidation: vi.fn(async () => false),
    },
  };
}

  it("cycles the four top-level tabs in both directions", () => {
    expect(nextDashboardTab("coordination", 1)).toBe("tools");
    expect(nextDashboardTab("tools", 1)).toBe("skills");
    expect(nextDashboardTab("skills", 1)).toBe("memory");
    expect(nextDashboardTab("memory", 1)).toBe("coordination");
    expect(nextDashboardTab("coordination", -1)).toBe("memory");
  });

  it("loads analytics on tab activation rather than dashboard construction", async () => {
    const usageLoader = vi.fn(async () => usage);
    const memory = memoryHarness();
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: usageLoader,
        memoryApi: memory.api,
        memoryPrompts: memory.prompts,
        done: vi.fn(),
      },
    );

    expect(usageLoader).not.toHaveBeenCalled();
    expect(component.render(80).join("\n")).toContain("Coordination");
    expect(component.render(80).join("\n")).toContain("Tools");
    expect(component.render(80).join("\n")).toContain("Skills");
    expect(component.render(80).join("\n")).toContain("Memory");

    component.handleInput("\t");
    await vi.waitFor(() => expect(usageLoader).toHaveBeenCalledWith(null, expect.any(AbortSignal)));
    await vi.waitFor(() => expect(component.render(80).join("\n")).toContain("bash"));
    component.dispose();
  });

  it("activates the local MCP memory pane only when its fourth tab is shown", async () => {
    const memory = memoryHarness();
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: async () => usage,
        memoryApi: memory.api,
        memoryPrompts: memory.prompts,
        done: vi.fn(),
      },
    );

    expect(memory.load).not.toHaveBeenCalled();
    component.handleInput("\t");
    component.handleInput("\t");
    component.handleInput("\t");

    await vi.waitFor(() => expect(memory.load).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(component.render(100).join("\n")).toContain("Keep boundaries typed"));
    expect(component.render(100).join("\n")).toContain("enter expand/detail");
    expect(component.render(100).join("\n")).toContain("f invalidate summary");
    expect(component.render(100).join("\n")).toContain("layered memory · live · MCP local only");
    expect(component.render(100).join("\n")).toContain("providers not yet attempted");
    expect(component.render(100).join("\n")).not.toContain("providers healthy");
    component.dispose();
  });
