// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import type { Theme } from "@earendil-works/pi-coding-agent";
import type { TUI } from "@earendil-works/pi-tui";
import { describe, expect, it, vi } from "vitest";
import {
  NexusDashboardComponent,
  nextDashboardTab,
} from "../src/component.ts";
import type { Snapshot, UsageSummary } from "../src/domain.ts";

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

describe("Pi dashboard tabs", () => {
  it("cycles the three top-level tabs in both directions", () => {
    expect(nextDashboardTab("coordination", 1)).toBe("tools");
    expect(nextDashboardTab("tools", 1)).toBe("skills");
    expect(nextDashboardTab("skills", 1)).toBe("coordination");
    expect(nextDashboardTab("coordination", -1)).toBe("skills");
  });

  it("loads analytics on tab activation rather than dashboard construction", async () => {
    const usageLoader = vi.fn(async () => usage);
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: usageLoader,
        done: vi.fn(),
      },
    );

    expect(usageLoader).not.toHaveBeenCalled();
    expect(component.render(80).join("\n")).toContain("Coordination");
    expect(component.render(80).join("\n")).toContain("Tools");
    expect(component.render(80).join("\n")).toContain("Skills");

    component.handleInput("\t");
    await vi.waitFor(() => expect(usageLoader).toHaveBeenCalledWith(null, expect.any(AbortSignal)));
    await vi.waitFor(() => expect(component.render(80).join("\n")).toContain("bash"));
    component.dispose();
  });
});
