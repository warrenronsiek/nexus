// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { describe, expect, it, vi } from "vitest";
import { AnalyticsBrowser, decodeAnalytics, type AnalyticsApi } from "../src/analytics.ts";

const data = {
  summary: { ok: true as const, observed_calls: 3, identified_actions: 2, classified_actions: 2, unclassified_actions: 0, activities: [{ category_id: "ci.inspect", name: "Inspect CI", calls: 2, actions: 2, sessions: 1, improvised_actions: 2 }], truncated: false, window_started_at: "2026-10-13", window_ended_at: "2026-10-20", counting: "Category call counts overlap." },
  catalog: { ok: true as const, capabilities: [{ resource_key: "tool:github.checks", name: "github.checks", kind: "tool", origin: "host_tool", calls: 0, offered_sessions: 3, discoverable_sessions: 0, availability: "observed_inventory", description: "Inspect CI checks" }], next_offset: null },
  opportunities: { ok: true as const, opportunities: [{ category_id: "ci.inspect", name: "Inspect CI", calls: 2, sessions: 1, supplied_alternatives: ["github.checks"], evidence: "Suitability remains unknown." }], truncated: false },
};
const api: AnalyticsApi = {
  catalog: vi.fn(async () => data.catalog),
  examples: vi.fn(async () => ({ ok: true as const, examples: [{ name: "Inline program", description: "Inspect CI checks", origin: "agent_authored", evidence: "model", tool_name: "Bash", call_outcome: "succeeded", model_id: "codex:configured-default", observed_at: "2026-10-20", category_id: "ci.inspect" }], next_offset: null })),
};

describe("activity analytics browser", () => {
  it("validates analytics counts and exposes supplied zero-use tools", () => {
    expect(decodeAnalytics(data)).toEqual(data);
    expect(() => decodeAnalytics({ ...data, summary: { ...data.summary, observed_calls: -1 } })).toThrow();
    const browser = new AnalyticsBrowser(api, () => undefined);
    browser.update(data, "project-1");
    expect(browser.render(90).join("\n")).toContain("Inspect CI");
    browser.handleInput("2");
    expect(browser.render(90).join("\n")).toContain("github.checks");
    expect(browser.render(90).join("\n")).toContain("0 calls");
    expect(browser.render(90).join("\n")).toContain("3 offered sessions");
  });
  it("discloses activity implementations on selection and sanitizes detail", async () => {
    const browser = new AnalyticsBrowser(api, () => undefined);
    browser.update(data, "project-1");
    browser.handleInput("\r");
    await vi.waitFor(() => expect(browser.render(90).join("\n")).toContain("agent_authored"));
    expect(api.examples).toHaveBeenCalledWith("project-1", { category_id: "ci.inspect", offset: 0 }, expect.any(AbortSignal));
    expect(browser.handleInput("\u001b")).toBe(true);
    expect(browser.render(90).join("\n")).toContain("Inspect CI");
    expect(browser.render(18).every((line) => line.length <= 18)).toBe(true);
  });
});

it("keeps late details from a previous scope out of the current view", async () => {
  let finish!: (value: Awaited<ReturnType<AnalyticsApi["examples"]>>) => void;
  const slow: AnalyticsApi = { ...api, examples: () => new Promise((resolve) => { finish = resolve; }) };
  const browser = new AnalyticsBrowser(slow, () => undefined);
  browser.update(data, "old"); browser.handleInput("\r");
  browser.update(data, "new");
  finish({ ok: true as const, examples: [{ name: "OLD-SCOPE", description: "Old", origin: "unknown", evidence: "model", tool_name: "Bash", call_outcome: "succeeded", model_id: null, observed_at: "now", category_id: "ci.inspect" }], next_offset: null });
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(browser.render(90).join("\n")).not.toContain("OLD-SCOPE");
});

it("keeps a full page of long capability names within a terminal and discloses names in detail", () => {
  const longName = "repository/scripts/" + "long-name-".repeat(12);
  const bundle = { ...data, catalog: { ...data.catalog, capabilities: Array.from({ length: 12 }, (_, index) => ({ ...data.catalog.capabilities[0], name: `${longName}${index}` })) } };
  const browser = new AnalyticsBrowser(api, () => undefined);
  browser.update(bundle, "project");browser.handleInput("2");
  expect(browser.render(40).length).toBeLessThanOrEqual(24);
  browser.handleInput("\r");
  expect(browser.render(40).join("")).toContain(longName);
  browser.deactivate();
});

it("leaves the inspection deadline to transport rather than aborting valid analytics reads at ten seconds", async () => {
  vi.useFakeTimers();
  try {
    const { AnalyticsRequests } = await import("../src/analytics-requests.ts");
    const requests = new AnalyticsRequests(vi.fn());
    let signal!: AbortSignal, finish!: (value: number) => void;
    const apply = vi.fn();
    requests.run(current => { signal = current; return new Promise<number>(resolve => { finish = resolve; }); }, apply);
    await vi.advanceTimersByTimeAsync(11_000);
    expect(signal.aborted).toBe(false);
    finish(42);
    await vi.advanceTimersByTimeAsync(0);
    expect(apply).toHaveBeenCalledWith(42);
    requests.cancel();
  } finally { vi.useRealTimers(); }
});
