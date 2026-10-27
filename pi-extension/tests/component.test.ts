// @feature work-history
// @spec docs/features/work-history.md
// @feature evals
// @spec docs/features/evals.md
// @feature observability-ui
// @feature usage-analytics
// @feature agent-memory
// @feature sos
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/sos.md
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
import type { SosApi, SosList } from "../src/sos.ts";

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
  truncated: false,
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

const sosList: SosList = {
  ok: true,
  issues: [],
  untriaged_report_count: 0,
  truncated: false,
};

function sosHarness(): { api: SosApi; list: ReturnType<typeof vi.fn> } {
  const list = vi.fn(async () => sosList);
  return {
    list,
    api: {
      list,
      get: vi.fn(async () => { throw new Error("no SOS issue selected"); }),
    },
  };
}

  it("cycles the eight top-level tabs in both directions", () => {
    expect(nextDashboardTab("coordination", 1)).toBe("sos");
    expect(nextDashboardTab("sos", 1)).toBe("tools");
    expect(nextDashboardTab("tools", 1)).toBe("skills");
    expect(nextDashboardTab("skills", 1)).toBe("memory");
    expect(nextDashboardTab("memory", 1)).toBe("papercuts");
    expect(nextDashboardTab("papercuts", 1)).toBe("evals");
    expect(nextDashboardTab("evals", 1)).toBe("work");
    expect(nextDashboardTab("work", 1)).toBe("coordination");
    expect(nextDashboardTab("coordination", -1)).toBe("work");
  });

  it("loads analytics on tab activation rather than dashboard construction", async () => {
    const usageLoader = vi.fn(async () => usage);
    const memory = memoryHarness();
    const sos = sosHarness();
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: usageLoader,
        papercutsApi: { list: async () => ({ papercuts: [], truncated: false }) },
        sosApi: sos.api,
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
    expect(component.render(80).join("\n")).toContain("SOS");

    component.handleInput("\t");
    expect(usageLoader).not.toHaveBeenCalled();
    component.handleInput("\t");
    await vi.waitFor(() => expect(usageLoader).toHaveBeenCalledWith(null, expect.any(AbortSignal)));
    await vi.waitFor(() => expect(component.render(80).join("\n")).toContain("bash"));
    component.dispose();
  });

  it("activates the local MCP memory pane only when its fourth tab is shown", async () => {
    const memory = memoryHarness();
    const sos = sosHarness();
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: async () => usage,
        papercutsApi: { list: async () => ({ papercuts: [], truncated: false }) },
        sosApi: sos.api,
        memoryApi: memory.api,
        memoryPrompts: memory.prompts,
        done: vi.fn(),
      },
    );

    expect(memory.load).not.toHaveBeenCalled();
    component.handleInput("\t");
    component.handleInput("\t");
    component.handleInput("\t");
    component.handleInput("\t");

    await vi.waitFor(() => expect(memory.load).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(component.render(100).join("\n")).toContain("Keep boundaries typed"));
    expect(component.render(100).join("\n")).toContain("enter expand/detail");
    expect(component.render(100).join("\n")).toContain("f invalidate summary");
    expect(component.render(100).join("\n")).toContain(
      "layered memory · live · MCP via configured companion",
    );
    expect(component.render(100).join("\n")).toContain("providers not yet attempted");
    expect(component.render(100).join("\n")).not.toContain("providers healthy");
    component.dispose();
  });

  it("loads the read-only SOS pane only when its tab is shown", async () => {
    const memory = memoryHarness();
    const sos = sosHarness();
    const component = new NexusDashboardComponent(
      { requestRender: vi.fn() } as unknown as TUI,
      theme,
      snapshot,
      {
        loadSnapshot: async () => snapshot,
        loadUsage: async () => usage,
        papercutsApi: { list: async () => ({ papercuts: [], truncated: false }) },
        sosApi: sos.api,
        memoryApi: memory.api,
        memoryPrompts: memory.prompts,
        done: vi.fn(),
      },
    );

    expect(sos.list).not.toHaveBeenCalled();
    component.handleInput("\t");
    await vi.waitFor(() => expect(sos.list).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(component.render(80).join("\n")).toContain("No current SOS issues"));
    expect(component.render(80).join("\n")).toContain("0 awaiting triage");
    component.dispose();
  });

it("opens papercuts from the shared dashboard tab bar", async () => {
  const memory = memoryHarness();
  const sos = sosHarness();
  const list = vi.fn(async () => ({ papercuts: [], truncated: false }));
  const component = new NexusDashboardComponent(
    { requestRender: vi.fn() } as unknown as TUI, theme, snapshot,
    { loadSnapshot: async () => snapshot, loadUsage: async () => usage,
      sosApi: sos.api, papercutsApi: { list }, memoryApi: memory.api,
      memoryPrompts: memory.prompts, done: vi.fn() },
  );
  expect(list).not.toHaveBeenCalled();
  for (let i = 0; i < 3; i++) component.handleInput("\u001b[Z");
  await vi.waitFor(() => expect(component.render(120).join("\n")).toContain("No open papercuts"));
  expect(list).toHaveBeenCalledWith(null, expect.any(AbortSignal), undefined);
  component.dispose();
});

it("opens read-only Evals lazily and lets filter typing consume global shortcut letters", async () => {
  const memory = memoryHarness();
  const sos = sosHarness();
  const list = vi.fn(async () => ({ records: [], next_cursor: null }));
  const dashboard = vi.fn(async () => ({ groups: [], selected_group_id: null, series: [], run_count: 0, result_count: 0, scanned_records: 0, source_limit: 4096, truncated: false, notices: [] }));
  const done = vi.fn();
  const component = new NexusDashboardComponent(
    { requestRender: vi.fn() } as unknown as TUI, theme, snapshot,
    { loadSnapshot: async () => snapshot, loadUsage: async () => usage,
      sosApi: sos.api, papercutsApi: { list: async () => ({ papercuts: [], truncated: false }) },
      memoryApi: memory.api, memoryPrompts: memory.prompts,
      evalsApi: { dashboard, list, get: vi.fn(), why: vi.fn() }, done },
  );
  expect(list).not.toHaveBeenCalled();
  expect(dashboard).not.toHaveBeenCalled();
  component.handleInput("\u001b[Z");
  component.handleInput("\u001b[Z");
  await vi.waitFor(() => expect(dashboard).toHaveBeenCalled());
  expect(list).not.toHaveBeenCalled();
  component.handleInput("b");
  await vi.waitFor(() => expect(component.render(120).join("\n")).toContain("No matching eval history"));
  component.handleInput("/");
  component.handleInput("q");
  component.handleInput("r");
  component.handleInput("\r");
  await vi.waitFor(() => expect(list).toHaveBeenLastCalledWith(null, { search: "qr", limit: 24 }, expect.any(AbortSignal)));
  expect(done).not.toHaveBeenCalled();
  component.dispose();
});

it("opens Work lazily and lets work filters consume global shortcut letters", async () => {
  const memory = memoryHarness(); const sos = sosHarness(); const done = vi.fn();
  const list = vi.fn(async () => ({ items: [], next_cursor: null }));
  const dashboard = vi.fn(async () => ({ work_session_id: null, selected_session: null, sessions: [], activities: [], session_duration: { count: 0, known_count: 0, total_seconds: 0, average_seconds: null }, activity_kinds: [], daily: [], activity_daily: [], scanned_subjects: 0, source_limit: 4096, truncated: false, notices: [] }));
  const component = new NexusDashboardComponent({ requestRender: vi.fn() } as unknown as TUI, theme, snapshot,
    { loadSnapshot: async () => snapshot, loadUsage: async () => usage, sosApi: sos.api,
      papercutsApi: { list: async () => ({ papercuts: [], truncated: false }) }, memoryApi: memory.api, memoryPrompts: memory.prompts,
      workApi: { dashboard, health: vi.fn(async () => ({ available: false })), list, get: vi.fn(), timeline: vi.fn(), activities: vi.fn(), events: vi.fn() }, done });
  expect(list).not.toHaveBeenCalled();
  expect(dashboard).not.toHaveBeenCalled();
  component.handleInput("\u001b[Z");
  await vi.waitFor(() => expect(dashboard).toHaveBeenCalled());
  expect(list).not.toHaveBeenCalled();
  component.handleInput("d");
  await vi.waitFor(() => expect(component.render(120).join("\n")).toContain("No matching work sessions"));
  expect(component.render(160).join("\n")).toContain("↑↓ task · ← parent → child · PgUp/PgDn time · H/L lanes");
  component.handleInput("/"); component.handleInput("q"); component.handleInput("r"); component.handleInput("\r");
  await vi.waitFor(() => expect(list).toHaveBeenLastCalledWith(null, { search: "qr", limit: 24 }, expect.any(AbortSignal)));
  expect(done).not.toHaveBeenCalled(); component.dispose();
});
