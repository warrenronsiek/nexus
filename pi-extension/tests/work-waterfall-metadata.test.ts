// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { expect, it } from "vitest";
import { waterfallMetadata } from "../src/work-waterfall-metadata.ts";
import type { WaterfallNode } from "../src/work-waterfall.ts";

function task(values: Partial<WaterfallNode> = {}): WaterfallNode {
  return { id: "test-2", title: "Integration tests", kind: "test", parent_activity_id: "build-1",
    state: "finished", outcome: "succeeded", started_at: "2026-10-27T10:00:00Z", finished_at: "2026-10-27T10:00:03Z",
    elapsed: { nanoseconds: "3000000001", method: "os_monotonic", clock_id: "boot-1" },
    queued_at: "2026-10-27T09:59:59Z", queue_elapsed: { nanoseconds: "1000000000", method: "os_monotonic", clock_id: "boot-1" },
    executor: "worker-4", accounting_role: "execution", gaps: ["missing_tool_inventory"], conflicts: ["outcome_conflict"], ...values };
}

it("describes selected task identity, lifecycle, exact timing, queue evidence, and accounting below the graph", () => {
  const lines = waterfallMetadata(task(), "activity"), text = lines.join("\n");
  expect(lines).toHaveLength(8);
  for (const evidence of ["Integration tests", "test", "activity:test-2", "build-1", "finished", "succeeded",
    "2026-10-27T10:00:00Z", "2026-10-27T10:00:03Z", "3.000000001s", "os_monotonic", "boot-1",
    "2026-10-27T09:59:59Z", "1.000000000s", "worker-4", "execution", "missing_tool_inventory", "outcome_conflict"])
    expect(text).toContain(evidence);
  expect(text).toMatch(/Kind: [^()]+ \(test\)/u);
});

it("keeps missing timing and executor evidence unknown instead of deriving duration or assuming an endpoint", () => {
  const lines = waterfallMetadata(task({ parent_activity_id: null, state: "running", outcome: null, started_at: null, finished_at: null,
    elapsed: null, queued_at: undefined, queue_elapsed: undefined, executor: null, accounting_role: undefined, gaps: [], conflicts: [] }), "session");
  expect(lines[1]).toContain("state running · outcome unknown");
  expect(lines[2]).toContain("session:test-2 · parent none (session)");
  expect(lines[3]).toBe("Source: start unknown · finish unknown");
  expect(lines[4]).toBe("Elapsed: unknown");
  expect(lines[5]).toBe("Queue: unknown · wait unknown");
  expect(lines[6]).toBe("Executor: unknown · accounting unknown");
  expect(lines[7]).toBe("Gaps: none reported · conflicts: none reported");
});

it("preserves every exact duration digit and reports the duration's method and unknown clock", () => {
  const digits = "9".repeat(80);
  const lines = waterfallMetadata(task({ elapsed: { nanoseconds: digits, method: "reported", clock_id: null },
    queue_elapsed: { nanoseconds: "0", method: "reported", clock_id: "queue-clock" } }), "activity");
  expect(lines[4]).toBe(`Elapsed: ${"9".repeat(71)}.999999999s · reported · clock unknown`);
  expect(lines[5]).toContain("0.000000000s · reported · clock queue-clock");
});

it("bounds previews and notices with explicit omissions while leaving source controls literal for rendering", () => {
  const literal = "QA\u001b[31m";
  expect(waterfallMetadata(task({ title: literal, parent_activity_id: null }), "activity")[0]).toBe(`Selected: ${literal}`);
  expect(waterfallMetadata(task({ parent_activity_id: null }), "activity")[2]).toContain("parent session root");
  const long = "x".repeat(16_384);
  const lines = waterfallMetadata(task({ title: long, kind: long, id: long, parent_activity_id: long, state: long,
    started_at: long, finished_at: long, queued_at: long, executor: long,
    elapsed: { nanoseconds: "1".repeat(81), method: long, clock_id: long },
    gaps: [long, long, "third", "fourth"], conflicts: [long, long, "third"] }), "activity");
  expect(lines).toHaveLength(8);
  expect(lines.join("\n").length).toBeLessThan(4096);
  expect(lines[4]).toBe("Elapsed: unknown [duration exceeds display bound]");
  expect(lines[7]).toContain("+2 omitted");
  expect(lines[7]).toContain("+1 omitted");
  expect(lines.join("\n")).toContain("[display omitted]");
  expect(lines.join("\n")).not.toContain("third");
});
