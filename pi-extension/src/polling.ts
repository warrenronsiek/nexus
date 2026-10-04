// @feature observability-ui
// @spec docs/features/observability-ui.md
import type { Snapshot } from "./domain.ts";

export type RefreshMode = "poll" | "manual" | "scope";
export type SnapshotLoader = (projectId: string | null, signal: AbortSignal) => Promise<Snapshot>;

export interface PollingUpdate {
  snapshot: Snapshot;
  status: string;
  mode: RefreshMode;
}

interface RefreshRequest {
  controller: AbortController;
  projectId: string | null;
  signal: AbortSignal;
  timeout: NodeJS.Timeout;
}

export class SnapshotPoller {
  private timer: NodeJS.Timeout | undefined;
  private controller: AbortController | undefined;
  private disposed = false;
  private scope: string | null;

  constructor(
    private snapshot: Snapshot,
    private readonly load: SnapshotLoader,
    private readonly update: (update: PollingUpdate) => void,
  ) {
    this.scope = snapshot.dashboard.project_id;
  }

  start(): void {
    this.schedule();
  }

  async refresh(projectId: string | null, mode: RefreshMode): Promise<void> {
    const request = this.beginRefresh(projectId, mode);
    if (request === undefined) return;
    if (mode !== "poll") this.emit("refreshing…", mode);
    try {
      const snapshot = await this.load(request.projectId, request.signal);
      if (!this.isCurrent(request)) return;
      this.acceptSnapshot(snapshot, request.projectId, mode);
    } catch (error) {
      if (!this.isCurrent(request) || request.controller.signal.aborted) return;
      const message = error instanceof Error ? error.message : String(error);
      this.emit(`stale · ${message}`, mode);
    } finally {
      this.finishRefresh(request);
    }
  }

  private beginRefresh(projectId: string | null, mode: RefreshMode): RefreshRequest | undefined {
    if (this.disposed || (mode === "poll" && this.controller !== undefined)) return;
    if (mode === "scope") this.scope = projectId;
    this.controller?.abort();
    this.clearTimer();
    const controller = new AbortController();
    const timeoutController = new AbortController();
    const timeout = setTimeout(() => {
      timeoutController.abort(new Error("Nexus refresh timed out after 5000ms"));
    }, 5000);
    timeout.unref();
    this.controller = controller;
    return {
      controller,
      projectId: this.scope,
      signal: AbortSignal.any([controller.signal, timeoutController.signal]),
      timeout,
    };
  }

  private isCurrent(request: RefreshRequest): boolean {
    return this.controller === request.controller;
  }

  private acceptSnapshot(snapshot: Snapshot, projectId: string | null, mode: RefreshMode): void {
    if (snapshot.dashboard.project_id !== projectId) {
      throw new Error("Nexus returned the wrong project scope");
    }
    this.snapshot = snapshot;
    this.emit("live", mode);
  }

  private finishRefresh(request: RefreshRequest): void {
    clearTimeout(request.timeout);
    if (this.isCurrent(request)) {
      this.controller = undefined;
      if (!this.disposed) this.schedule();
    }
  }

  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    this.controller?.abort();
  }

  private emit(status: string, mode: RefreshMode): void {
    this.update({ snapshot: this.snapshot, status, mode });
  }

  private schedule(): void {
    if (this.disposed) return;
    const delay = Math.max(500, this.snapshot.dashboard.refresh_interval_ms);
    this.timer = setTimeout(() => {
      void this.refresh(this.scope, "poll");
    }, delay);
    this.timer.unref();
  }

  private clearTimer(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
