// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import type { Snapshot, UsageSummary } from "./domain.ts";

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
      const snapshot = await this.load(request.projectId, request.controller.signal);
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
    this.controller = controller;
    return {
      controller,
      projectId: this.scope,
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

export type UsageLoader = (
  projectId: string | null,
  signal: AbortSignal,
) => Promise<UsageSummary>;

export interface UsagePollingUpdate {
  usage: UsageSummary | undefined;
  status: string;
}

interface UsageRefreshRequest {
  controller: AbortController;
  projectId: string | null;
}

export class UsagePoller {
  private timer: NodeJS.Timeout | undefined;
  private controller: AbortController | undefined;
  private active = false;
  private disposed = false;
  private scope: string | null = null;
  private usage: UsageSummary | undefined;

  constructor(
    private readonly load: UsageLoader,
    private readonly update: (update: UsagePollingUpdate) => void,
    private readonly intervalMs = 30_000,
  ) {}

  activate(projectId: string | null): void {
    if (this.disposed) return;
    this.active = true;
    this.scope = projectId;
    void this.refresh("loading…");
  }

  deactivate(): void {
    this.active = false;
    this.clearTimer();
    this.controller?.abort();
    this.controller = undefined;
  }

  setScope(projectId: string | null): void {
    if (this.scope === projectId) return;
    this.scope = projectId;
    this.usage = undefined;
    if (this.active) void this.refresh("loading…");
  }

  manualRefresh(): void {
    if (this.active) void this.refresh("refreshing…");
  }

  dispose(): void {
    this.disposed = true;
    this.deactivate();
  }

  private async refresh(pendingStatus?: string): Promise<void> {
    const request = this.beginRefresh(pendingStatus);
    if (request === undefined) return;
    try {
      const usage = await this.load(request.projectId, request.controller.signal);
      this.acceptUsage(request, usage);
    } catch (error) {
      this.failRefresh(request, error);
    } finally {
      this.finishRefresh(request);
    }
  }

  private beginRefresh(pendingStatus?: string): UsageRefreshRequest | undefined {
    if (!this.active || this.disposed) return;
    if (this.controller !== undefined && pendingStatus === undefined) return;
    this.controller?.abort();
    this.clearTimer();
    const controller = new AbortController();
    this.controller = controller;
    if (pendingStatus !== undefined) this.emit(pendingStatus);
    return {
      controller,
      projectId: this.scope,
    };
  }

  private acceptUsage(request: UsageRefreshRequest, usage: UsageSummary): void {
    if (!this.isCurrent(request)) return;
    if (usage.project_id !== request.projectId) {
      throw new Error("Nexus returned the wrong project scope");
    }
    this.usage = usage;
    this.emit("live");
  }

  private failRefresh(request: UsageRefreshRequest, error: unknown): void {
    if (!this.isCurrent(request) || request.controller.signal.aborted) return;
    const message = error instanceof Error ? error.message : String(error);
    this.emit(`stale · ${message}`);
  }

  private finishRefresh(request: UsageRefreshRequest): void {
    if (!this.isCurrent(request)) return;
    this.controller = undefined;
    this.schedule();
  }

  private isCurrent(request: UsageRefreshRequest): boolean {
    return this.controller === request.controller;
  }

  private emit(status: string): void {
    this.update({ usage: this.usage, status });
  }

  private schedule(): void {
    if (!this.active || this.disposed) return;
    this.timer = setTimeout(() => void this.refresh(), this.intervalMs);
    this.timer.unref();
  }

  private clearTimer(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
