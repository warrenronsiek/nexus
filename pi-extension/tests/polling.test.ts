// @feature observability-ui
// @spec docs/features/observability-ui.md
import { afterEach, expect, it, vi } from "vitest";
import type { Snapshot } from "../src/domain.ts";
import { SnapshotPoller } from "../src/polling.ts";

const snapshot = {
  dashboard: {
    project_id: null,
    refresh_interval_ms: 2000,
  },
} as Snapshot;

afterEach(() => {
  vi.useRealTimers();
});

  it("does not overlap requests and keeps the last snapshot after failure", async () => {
    vi.useFakeTimers();
    let rejectRequest: ((error: Error) => void) | undefined;
    const load = vi.fn(
      () =>
        new Promise<Snapshot>((_resolve, reject) => {
          rejectRequest = reject;
        }),
    );
    const updates = vi.fn();
    const poller = new SnapshotPoller(snapshot, load, updates);
    poller.start();

    await vi.advanceTimersByTimeAsync(2000);
    expect(load).toHaveBeenCalledTimes(1);
    await poller.refresh(null, "poll");
    expect(load).toHaveBeenCalledTimes(1);

    rejectRequest?.(new Error("offline"));
    await vi.runAllTicks();
    expect(updates).toHaveBeenLastCalledWith({
      snapshot,
      status: "stale · offline",
      mode: "poll",
    });
    poller.dispose();
  });

  it("times out a stalled request and schedules the next poll", async () => {
    vi.useFakeTimers();
    const load = vi.fn(
      (_projectId: string | null, signal: AbortSignal) =>
        new Promise<Snapshot>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(signal.reason), { once: true });
        }),
    );
    const updates = vi.fn();
    const poller = new SnapshotPoller(snapshot, load, updates);
    poller.start();

    await vi.advanceTimersByTimeAsync(2000);
    expect(load).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(updates.mock.calls.at(-1)?.[0]).toMatchObject({
      snapshot,
      mode: "poll",
    });
    expect(updates.mock.calls.at(-1)?.[0].status).toMatch(/^stale ·/u);
    await vi.advanceTimersByTimeAsync(2000);
    expect(load).toHaveBeenCalledTimes(2);
    poller.dispose();
  });

  it("lets a scope change preempt a background poll and keeps the requested scope", async () => {
    vi.useFakeTimers();
    const requests: Array<{
      projectId: string | null;
      signal: AbortSignal;
      resolve: (value: Snapshot) => void;
    }> = [];
    const load = vi.fn(
      (projectId: string | null, signal: AbortSignal) =>
        new Promise<Snapshot>((resolve) => requests.push({ projectId, signal, resolve })),
    );
    const poller = new SnapshotPoller(snapshot, load, vi.fn());
    poller.start();

    await vi.advanceTimersByTimeAsync(2000);
    const scopeRefresh = poller.refresh("project-1", "scope");
    await vi.runAllTicks();
    expect(load).toHaveBeenCalledTimes(2);
    expect(requests[0].signal.aborted).toBe(true);
    expect(requests[1].projectId).toBe("project-1");

    const scopedSnapshot = {
      dashboard: { ...snapshot.dashboard, project_id: "project-1" },
    } as Snapshot;
    requests[1].resolve(scopedSnapshot);
    await scopeRefresh;
    await vi.advanceTimersByTimeAsync(2000);
    expect(load).toHaveBeenLastCalledWith("project-1", expect.any(AbortSignal));
    poller.dispose();
  });
