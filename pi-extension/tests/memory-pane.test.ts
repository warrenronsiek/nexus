// @feature agent-memory
// @feature observability-ui
// @spec docs/features/agent-memory.md
// @spec docs/features/observability-ui.md
import type { Theme } from "@earendil-works/pi-coding-agent";
import type { TUI } from "@earendil-works/pi-tui";
import { beforeEach, expect, it, vi } from "vitest";
import {
  MemoryPane,
  type MemoryInteractionPrompts,
} from "../src/memory-pane.ts";
import type {
  MemoryApi,
  MemoryEntry,
  MemoryNode,
  MemoryView,
} from "../src/memory.ts";

function deferred<T>(): {
  promise: Promise<T>;
  resolve(value: T): void;
} {
  let resolvePromise: ((value: T) => void) | undefined;
  return {
    promise: new Promise<T>((resolve) => {
      resolvePromise = resolve;
    }),
    resolve(value: T) {
      resolvePromise?.(value);
    },
  };
}

const projectScope = { scope: "project", project_id: "project-1" } as const;
const provenance = {
  agent: "pi",
  session_id: "session-1",
  model_id: "model-1",
  config_hash: "config-1",
};
const raw: MemoryEntry = {
  kind: "raw",
  id: "raw-1",
  scope: projectScope,
  ordinal: 2,
  content: "Prefer typed interfaces.",
  content_hash: "hash-raw",
  provenance,
  created_at: "2026-10-04T12:00:00Z",
};
const summary: MemoryNode = {
  kind: "summary",
  id: "summary-1",
  scope: projectScope,
  level: 1,
  start_ordinal: 0,
  end_ordinal: 2,
  content: "The project favors narrow typed boundaries.",
  source_hash: "hash-summary",
  provider: "codex",
  model_id: "gpt-test",
  created_at: "2026-10-04T11:00:00Z",
};
const view: MemoryView = {
  context: {
    scope: { scope: "layered", project_id: "project-1" },
    nodes: [summary, raw],
    omitted: [],
    item_count: 2,
    byte_count: 82,
  },
  health: {
    scopes: [{ scope: projectScope, raw_entries: 3, summaries: 1 }],
    providers: [
      {
        provider: "codex",
        model_id: "gpt-test",
        last_outcome: "succeeded",
        last_attempted_at: "2026-10-04T12:04:00Z",
        next_retry_at: null,
        consecutive_failures: 0,
        cooling_down: false,
      },
      {
        provider: "claude",
        model_id: null,
        last_outcome: "failed",
        last_attempted_at: "2026-10-04T12:03:00Z",
        next_retry_at: "2026-10-04T12:08:00Z",
        consecutive_failures: 2,
        cooling_down: true,
      },
    ],
    pending_summaries: 2,
    failed_attempts: 1,
    cooling_down_attempts: 0,
    fallback_uses: 1,
    degraded: false,
    last_activation_at: "2026-10-04T12:05:00Z",
  },
};

const theme = {
  fg: (_color: string, text: string) => text,
  bold: (text: string) => text,
} as Theme;

function harness(
  overrides: Partial<MemoryInteractionPrompts> = {},
  loadedView: MemoryView = view,
) {
  const api = {
    load: vi.fn(async () => loadedView),
    expand: vi.fn(async () => [raw]),
    search: vi.fn(async () => [raw]),
    add: vi.fn(async () => undefined),
    invalidate: vi.fn(async () => undefined),
    consolidate: vi.fn(async () => undefined),
  };
  const prompts: MemoryInteractionPrompts = {
    searchRegex: vi.fn(async () => "typed.*interfaces"),
    newMemory: vi.fn(async () => ({
      scope: "global" as const,
      content: "Keep releases reversible.",
    })),
    confirmInvalidation: vi.fn(async () => true),
    ...overrides,
  };
  const tui = { requestRender: vi.fn() } as unknown as TUI;
  const pane = new MemoryPane(tui, theme, api, prompts);
  return { api, pane, prompts, tui };
}

async function activate(pane: MemoryPane): Promise<void> {
  pane.activate();
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Prefer typed"));
}

  beforeEach(() => vi.restoreAllMocks());

  it("shows nodes, scope counts, pending work, provider health, fallback use, and activation health", async () => {
    const scopedView: MemoryView = {
      ...view,
      context: {
        ...view.context,
        omitted: [
          { scope: projectScope, start_ordinal: 0, end_ordinal: 3, reason: "mixed" },
          { scope: projectScope, start_ordinal: 4, end_ordinal: 7, reason: "budget" },
          { scope: projectScope, start_ordinal: 8, end_ordinal: 11, reason: "budget" },
          { scope: projectScope, start_ordinal: 12, end_ordinal: 15, reason: "budget" },
          { scope: projectScope, start_ordinal: 16, end_ordinal: 19, reason: "budget" },
        ],
      },
      health: {
        ...view.health,
        scopes: [
          { scope: { scope: "global" }, raw_entries: 5, summaries: 2 },
          ...view.health.scopes,
        ],
      },
    };
    const { pane } = harness({}, scopedView);
    await activate(pane);
    const rendered = pane.render(120).join("\n");

    expect(rendered).toContain("global · 5 raw · 2 summaries");
    expect(rendered).toContain("project · 3 raw · 1 summary");
    expect(rendered).toContain("2 pending");
    expect(rendered).toContain("providers healthy");
    expect(rendered).toContain("provider codex");
    expect(rendered).toContain("codex · gpt-test · succeeded · ready · failures 0");
    expect(rendered).toContain("claude · default model · failed · cooling until");
    expect(rendered).toContain("fallbacks 1");
    expect(rendered).toContain("activated");
    expect(rendered).toContain("omitted project #0-3 · mixed");
    expect(rendered).toContain("2 more omitted ranges");
    expect(rendered).not.toContain("omitted project #16-19");
    pane.dispose();
  });

  it("shows failed provider health even when the memory context is empty", async () => {
    const emptyView: MemoryView = {
      ...view,
      context: { ...view.context, nodes: [], item_count: 0, byte_count: 0 },
      health: { ...view.health, degraded: true },
    };
    const { pane } = harness({}, emptyView);
    pane.activate();
    await vi.waitFor(() => expect(pane.render(120).join("\n")).toContain("No memory nodes"));

    const rendered = pane.render(120).join("\n");
    expect(rendered).toContain("providers degraded");
    expect(rendered).toContain("claude · default model · failed · cooling until");
    pane.dispose();
  });

  it("uses Enter to expand summaries and show raw-note detail", async () => {
    const { api, pane } = harness();
    await activate(pane);

    pane.handleInput("\r");
    await vi.waitFor(() => expect(api.expand).toHaveBeenCalledWith("summary-1"));
    await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Expanded summary"));

    pane.handleInput("\r");
    await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Raw memory detail"));
    expect(pane.render(100).join("\n")).toContain("session-1");
    pane.dispose();
  });

  it("maps slash search, add, consolidate, and refresh to typed memory actions", async () => {
    const { api, pane } = harness();
    await activate(pane);
    expect(api.load).toHaveBeenCalledTimes(1);

    pane.handleInput("/");
    await vi.waitFor(() => expect(api.search).toHaveBeenCalledWith("typed.*interfaces"));

    pane.handleInput("a");
    await vi.waitFor(() => expect(api.add).toHaveBeenCalledWith(
      "global",
      "Keep releases reversible.",
    ));

    pane.handleInput("c");
    await vi.waitFor(() => expect(api.consolidate).toHaveBeenCalledOnce());

    pane.handleInput("r");
    await vi.waitFor(() => expect(api.load.mock.calls.length).toBeGreaterThanOrEqual(4));
    pane.dispose();
  });

  it("keeps the selected node visible within a bounded long-history viewport", async () => {
    const nodes = Array.from({ length: 24 }, (_, index): MemoryEntry => ({
      ...raw,
      id: `raw-${index}`,
      ordinal: index,
      content: `memory-${index}`,
    }));
    const longView: MemoryView = {
      ...view,
      context: {
        ...view.context,
        nodes,
        item_count: nodes.length,
      },
    };
    const { pane } = harness({}, longView);
    pane.activate();
    await vi.waitFor(() => expect(pane.render(80).join("\n")).toContain("memory-0"));

    for (let index = 0; index < 18; index += 1) pane.handleInput("\u001b[B");

    const rendered = pane.render(80).join("\n");
    expect(rendered).toContain("memory-18");
    expect(rendered).not.toContain("memory-0");
    expect(rendered).toContain("older nodes above");
    pane.dispose();
  });

  it("wraps long raw details and lets navigation reveal the complete note", async () => {
    const longRaw: MemoryEntry = {
      ...raw,
      content: `${"segment ".repeat(45)}TAIL-MARKER`,
    };
    const detailView: MemoryView = {
      ...view,
      context: { ...view.context, nodes: [longRaw], item_count: 1 },
    };
    const { pane } = harness({}, detailView);
    pane.activate();
    await vi.waitFor(() => expect(pane.render(24).join("\n")).toContain("Layered memory"));
    pane.handleInput("\r");

    expect(pane.render(24).join("\n")).toContain("Raw memory detail");
    expect(pane.render(24).join("\n")).not.toContain("TAIL-MARKER");
    for (let index = 0; index < 30; index += 1) pane.handleInput("\u001b[B");
    expect(pane.render(24).join("\n")).toContain("TAIL-MARKER");
    pane.dispose();
  });

  it("ignores an action completion from an earlier tab activation", async () => {
    const consolidation = deferred<undefined>();
    const initialRaw = { ...raw, id: "initial", content: "initial view" };
    const currentRaw = { ...raw, id: "current", content: "current view" };
    const staleRaw = { ...raw, id: "stale", content: "stale completion" };
    const initialView = { ...view, context: { ...view.context, nodes: [initialRaw] } };
    const currentView = { ...view, context: { ...view.context, nodes: [currentRaw] } };
    const staleView = { ...view, context: { ...view.context, nodes: [staleRaw] } };
    const { api, pane } = harness({}, initialView);
    api.load
      .mockReset()
      .mockResolvedValueOnce(initialView)
      .mockResolvedValueOnce(currentView)
      .mockResolvedValue(staleView);
    api.consolidate.mockImplementationOnce(() => consolidation.promise);
    pane.activate();
    await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("initial view"));
    pane.handleInput("c");
    await vi.waitFor(() => expect(api.consolidate).toHaveBeenCalledOnce());

    pane.deactivate();
    pane.activate();
    await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("current view"));
    consolidation.resolve(undefined);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(api.load).toHaveBeenCalledTimes(2);
    expect(pane.render(100).join("\n")).toContain("current view");
    expect(pane.render(100).join("\n")).not.toContain("stale completion");
    pane.dispose();
  });

  it("serializes repeated consolidation requests while one is in flight", async () => {
    const consolidation = deferred<undefined>();
    const { api, pane } = harness();
    api.consolidate.mockImplementation(() => consolidation.promise);
    await activate(pane);

    pane.handleInput("c");
    pane.handleInput("c");

    await vi.waitFor(() => expect(api.consolidate).toHaveBeenCalledOnce());
    consolidation.resolve(undefined);
    await vi.waitFor(() => expect(api.load).toHaveBeenCalledTimes(2));
    pane.dispose();
  });

  it("confirms summary invalidation and never invalidates a raw note", async () => {
    const { api, pane, prompts } = harness();
    await activate(pane);

    pane.handleInput("f");
    await vi.waitFor(() => expect(prompts.confirmInvalidation).toHaveBeenCalledWith(summary));
    await vi.waitFor(() => expect(api.invalidate).toHaveBeenCalledWith("summary-1"));

    pane.handleInput("\u001b[B");
    pane.handleInput("f");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(api.invalidate).toHaveBeenCalledTimes(1);
    pane.dispose();
  });

  it("does not invalidate a summary when confirmation is declined", async () => {
    const { api, pane, prompts } = harness({ confirmInvalidation: vi.fn(async () => false) });
    await activate(pane);
    pane.handleInput("f");
    await vi.waitFor(() => expect(prompts.confirmInvalidation).toHaveBeenCalledOnce());
    expect(api.invalidate).not.toHaveBeenCalled();
    pane.dispose();
  });
