// @feature observability-ui
// @spec docs/features/observability-ui.md
import { afterEach, expect, it, vi } from "vitest";
import type { TUI } from "@earendil-works/pi-tui";
import { NexusDashboardConnection } from "../src/connection.ts";
import type { DashboardOptions } from "../src/component.ts";
import type { Snapshot } from "../src/domain.ts";
import type { SnapshotLoader } from "../src/polling.ts";

const theme = { bold: (text: string) => text, fg: (_color: string, text: string) => text };
const snapshot = {
  dashboard: { ok: true, project_id: "project", generated_at: "2026-10-29T12:00:00Z", refresh_interval_ms: 2_000,
    counts: { active_sessions: 7, active_claims: 0, open_conflicts: 0, events: 7 },
    events: { items: [], truncated: false }, sessions: { items: [], truncated: false },
    claims: { items: [], truncated: false }, conflicts: { items: [], truncated: false } }, projects: [],
} satisfies Snapshot;

function options(loadSnapshot: SnapshotLoader, done: () => void): DashboardOptions {
  const unused = async (): Promise<never> => { throw new Error("Inactive pane must not load"); };
  return { loadSnapshot, done, loadUsage: unused, papercutsApi: { list: unused }, sosApi: { list: unused, get: unused },
    memoryApi: { load: unused, search: unused, expand: unused, add: unused, invalidate: unused, consolidate: unused },
    memoryPrompts: { searchRegex: unused, newMemory: unused, confirmInvalidation: unused } };
}
function tui(): TUI { return { requestRender: vi.fn() } as unknown as TUI; }

afterEach(() => { vi.useRealTimers(); });

it("opens while connecting, stays single flight, and ignores a result after host disposal", async () => {
  let finish!: (value: Snapshot) => void;
  const load = vi.fn((_project: string | null, _signal: AbortSignal) => new Promise<Snapshot>((resolve) => { finish = resolve; }));
  let connection!: NexusDashboardConnection;
  const done = vi.fn(() => connection.dispose());
  connection = new NexusDashboardConnection(tui(), theme, options(load, done));
  expect(connection.render(100).join("\n")).toContain("Connecting to Nexus");
  expect(connection.render(100).join("\n")).not.toContain("recent sessions");
  connection.handleInput("r");
  expect(load).toHaveBeenCalledTimes(1);
  connection.handleInput("q");
  expect(done).toHaveBeenCalledOnce();
  expect(load.mock.calls[0][1].aborted).toBe(true);
  finish(snapshot);
  await Promise.resolve();
  expect(connection.render(100).join("\n")).not.toContain("Coordination");
});

it("keeps the timeout visible and retries reads before showing the real dashboard", async () => {
  vi.useFakeTimers();
  const load = vi.fn().mockRejectedValueOnce(new Error("Nexus MCP request timed out after 5000ms")).mockResolvedValueOnce(snapshot);
  let connection!: NexusDashboardConnection;
  const done = vi.fn(() => connection.dispose());
  connection = new NexusDashboardConnection(tui(), theme, options(load, done));
  await vi.advanceTimersByTimeAsync(0);
  expect(connection.render(100).join("\n")).toContain("timed out after 5000ms");
  expect(connection.render(100).join("\n")).not.toContain("recent sessions");
  expect(done).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(2_000);
  expect(load).toHaveBeenCalledTimes(2);
  expect(connection.render(100).join("\n")).toContain("Coordination");
  expect(connection.render(100).join("\n")).toContain("7 recent sessions");
  connection.invalidate();
  connection.handleInput("q");
  expect(done).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(load).toHaveBeenCalledTimes(2);
});

it("allows immediate retry and cancels scheduled retry on disposal", async () => {
  vi.useFakeTimers();
  const load = vi.fn().mockRejectedValue(new Error("remote\u001b[31m unavailable"));
  const connection = new NexusDashboardConnection(tui(), theme, options(load, vi.fn()));
  await vi.advanceTimersByTimeAsync(0);
  expect(connection.render(100).join("\n")).not.toContain("\u001b[31m");
  connection.handleInput("r");
  await vi.advanceTimersByTimeAsync(0);
  expect(load).toHaveBeenCalledTimes(2);
  connection.dispose();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(load).toHaveBeenCalledTimes(2);
});
