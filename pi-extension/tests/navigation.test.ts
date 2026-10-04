// @feature observability-ui
// @spec docs/features/observability-ui.md
import { describe, expect, it } from "vitest";
import type { Snapshot } from "../src/domain.ts";
import { entriesFor, selectEntry, type Page } from "../src/navigation.ts";

const snapshot: Snapshot = {
  dashboard: {
    ok: true,
    project_id: null,
    generated_at: "2026-09-30T12:00:00Z",
    refresh_interval_ms: 2000,
    counts: {
      active_sessions: 2,
      active_claims: 3,
      open_conflicts: 1,
      events: 4,
    },
    conflicts: {
      truncated: false,
      items: [
        {
          id: "conflict-1",
          project_id: "project-1",
          left_claim_id: "claim-1",
          right_claim_id: "claim-2",
          path: "src/main.rs",
          severity: "critical",
          kind: "hunk_overlap",
          status: "open",
          message: "overlapping lines",
          created_at: "2026-09-30T11:59:00Z",
          updated_at: "2026-09-30T11:59:30Z",
        },
      ],
    },
    sessions: {
      truncated: false,
      items: [
        {
          session_id: "session-1",
          project_id: "project-1",
          agent: "codex",
          worktree: "/repo",
          status: "active",
          task_summary: "Implement terminal view",
          last_seen_at: "2026-09-30T11:59:30Z",
        },
      ],
    },
    claims: { truncated: false, items: [] },
    events: {
      truncated: false,
      items: [
        {
          id: 42,
          project_id: "project-1",
          session_id: "session-1",
          kind: "conflict_detected",
          payload: { severity: "critical" },
          created_at: "2026-09-30T11:59:00Z",
        },
      ],
    },
  },
  projects: [
    {
      project_id: "project-1",
      worktrees: ["/repo"],
      agents: ["codex"],
      last_seen_at: "2026-09-30T11:59:30Z",
    },
  ],
};

describe("progressive navigation", () => {
  it("reveals summary, list, and detail one level at a time", () => {
    const overview: Page = { kind: "overview" };
    expect(entriesFor(snapshot, overview).map((entry) => entry.label)).toEqual([
      "Project scope",
      "Conflicts  1",
      "Sessions   2",
      "Claims     3",
      "Events     4",
    ]);

    const conflicts = selectEntry(snapshot, overview, "section:conflicts");
    const conflictsPage = { kind: "list", section: "conflicts" } as const;
    expect(conflicts).toEqual({ kind: "navigate", page: conflictsPage });
    expect(entriesFor(snapshot, conflictsPage)[0]).toMatchObject({
      label: "CRITICAL  src/main.rs",
      description: "overlapping lines",
    });

    const detail = selectEntry(snapshot, { kind: "list", section: "conflicts" }, "record:conflict-1");
    expect(detail).toEqual({
      kind: "navigate",
      page: { kind: "detail", section: "conflicts", recordKey: "conflict-1" },
    });
  });

  it("returns a project-scope action without mutating navigation state", () => {
    const result = selectEntry(snapshot, { kind: "list", section: "projects" }, "project:project-1");
    expect(result).toEqual({ kind: "scope", projectId: "project-1" });
  });
});
