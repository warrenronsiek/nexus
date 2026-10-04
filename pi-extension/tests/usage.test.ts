// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { describe, expect, it } from "vitest";
import { decodeUsage, renderUsageBars } from "../src/usage.ts";

const response = {
  ok: true,
  project_id: "project-1",
  window_started_at: "2026-09-27T12:00:00Z",
  window_ended_at: "2026-10-04T12:00:00Z",
  tools: [
    {
      kind: "tool",
      name: "bash",
      count: 12,
      sessions: 4,
      succeeded: 10,
      failed: 1,
      observed: 1,
    },
    {
      kind: "script",
      name: "scripts/check.sh",
      count: 3,
      sessions: 2,
    },
  ],
  skills: [
    {
      kind: "skill",
      name: "tdd",
      count: 5,
      sessions: 3,
      evidence: { explicit_invocation: 4, instruction_read: 1 },
    },
  ],
  capture_health: { pending: 1, dropped: 0 },
};

describe("usage analytics boundary", () => {
  it("decodes the seven-day usage response while preserving optional detail", () => {
    expect(decodeUsage(response)).toEqual(response);
    expect(() =>
      decodeUsage({ ...response, tools: [{ ...response.tools[0], count: -1 }] }),
    ).toThrow("unexpected response");
  });

  it("renders responsive bars with counts, sessions, and script identity", () => {
    const usage = decodeUsage(response);
    const lines = renderUsageBars(usage.tools, 48);

    expect(lines).toHaveLength(2);
    expect(lines[0]).toContain("bash");
    expect(lines[0]).toContain("12 · 4 sessions");
    expect(lines[1]).toContain("[script] scripts/check.sh");
    expect(lines.every((line) => line.length <= 48)).toBe(true);
    expect((lines[0].match(/█/gu) ?? []).length).toBeGreaterThan(
      (lines[1].match(/█/gu) ?? []).length,
    );
  });

  it("keeps narrow and empty views legible", () => {
    const usage = decodeUsage(response);
    expect(renderUsageBars([], 40)).toEqual(["No usage observed in this window."]);
    expect(renderUsageBars(usage.tools, 28).every((line) => line.length <= 28)).toBe(true);
  });
});
