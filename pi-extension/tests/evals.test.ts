// @feature evals
// @spec docs/features/evals.md
// @feature observability-ui
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";
import { NexusEvalsApi } from "../src/evals.ts";
import { EvalsPane } from "../src/evals-pane.ts";
import { evalDetailLines } from "../src/evals-view.ts";

const header = {
  id: "change-1", kind: "change" as const, title: "Tool discovery regression",
  occurred_at: "2026-10-22T12:00:00Z", recorded_at: "2026-10-22T12:01:00Z",
  repository: "nexus", model: "model-a",
};
const record = {
  id: header.id, title: header.title, occurred_at: header.occurred_at,
  recorded_at: header.recorded_at, project_id: "project-1",
  provenance: { agent: "test", session_id: null, model_id: null, config_hash: null },
  body: { kind: "change" as const, motivation: {
    state: "recorded", reason: "observed_failure", problem: "Historical failure\u001b]52;c;secret\u0007: tool was discoverable but ignored.",
    expected_behavior: "Discover the tool", rationale: "Reproduce the observed missed capability", provenance: "contemporaneous",
  }, evidence: [] },
  links: [{ relation: "evidence", target_id: "trial-1" }],
};
const theme = { fg: (_color: string, text: string) => text, bold: (text: string) => text };

it("sends bounded eval history filters and validates records at the MCP boundary", async () => {
  const call = vi.fn().mockResolvedValueOnce({ ok: true, action: "listed", records: [header], next_cursor: "older" })
    .mockResolvedValueOnce({ ok: true, action: "detailed", record })
    .mockResolvedValueOnce({ ok: true, action: "history", records: [record], truncated: true });
  const api = new NexusEvalsApi({ call }, "/repo");
  const filters = { kind: "change" as const, repository: "nexus", model: "model-a", search: "discovery", after: "cursor", limit: 24 };
  expect((await api.list("project-1", filters)).next_cursor).toBe("older");
  expect(call.mock.calls[0]).toEqual(["nexus_evals", { action: "list", project_id: "project-1", ...filters }, "/repo", undefined]);
  expect((await api.get("project-1", "change-1"))?.body.kind).toBe("change");
  expect((await api.why("project-1", "change-1")).truncated).toBe(true);
  const malformed = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "listed", records: [{ ...header, kind: "mystery" }], next_cursor: null })) }, "/repo");
  await expect(malformed.list(null, {})).rejects.toThrow("unexpected response");
});

it("progressively opens rationale and linked evidence without loading artifacts", async () => {
  const list = vi.fn(async () => ({ records: [header], next_cursor: "older" }));
  const get = vi.fn(async () => record);
  const why = vi.fn(async () => ({ records: [record], truncated: true }));
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme, { list, get, why }, vi.fn());
  expect(list).not.toHaveBeenCalled();
  pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(pane.render(100).join("\n")).toContain("1 records loaded");
  expect(pane.render(100).join("\n")).toContain("more records");
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Historical failure"));
  pane.handleInput("l");
  expect(pane.render(100).join("\n")).toContain("trial-1");
  expect(pane.render(100).join("\n")).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u);
  pane.handleInput("\u001b");
  pane.handleInput("w");
  await vi.waitFor(() => expect(why).toHaveBeenCalledWith("project-1", "change-1", expect.any(AbortSignal)));
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("History is partial"));
  for (const width of [20, 40, 80]) expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
  pane.dispose();
});

it("accepts trace, artifact-verification, and lifecycle headers in a mixed history page", async () => {
  const records = ["trace", "verification", "lifecycle"].map(kind => ({ ...header, id: kind, kind }));
  const api = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "listed", records, next_cursor: null })) }, "/repo");
  expect((await api.list(null, {})).records.map(record => record.kind)).toEqual(["trace", "verification", "lifecycle"]);
});

it("aborts superseded eval scope reads and retains successful data on refresh failure", async () => {
  let complete!: (value: { records: typeof header[]; next_cursor: null }) => void;
  const delayed = new Promise<{ records: typeof header[]; next_cursor: null }>(resolve => { complete = resolve; });
  const list = vi.fn().mockReturnValueOnce(delayed)
    .mockResolvedValueOnce({ records: [{ ...header, title: "New project history" }], next_cursor: null })
    .mockRejectedValue(new Error("offline"));
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list, get: vi.fn(), why: vi.fn() }, vi.fn());
  pane.activate("old");
  const signal = list.mock.calls[0][2] as AbortSignal;
  pane.deactivate();
  expect(signal.aborted).toBe(true);
  pane.activate("new");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("New project history"));
  complete({ records: [header], next_cursor: null });
  await delayed;
  expect(pane.render(100).join("\n")).not.toContain(header.title);
  pane.refresh();
  await vi.waitFor(() => expect(pane.statusLine()).toContain("stale"));
  expect(pane.render(100).join("\n")).toContain("New project history");
  pane.dispose();
});

it("loads bounded next pages and displays exact Git coordinates without fetching artifact bytes", async () => {
  const list = vi.fn().mockResolvedValueOnce({ records: [header], next_cursor: "older" })
    .mockResolvedValueOnce({ records: [{ ...header, id: "artifact-1", kind: "artifact", title: "Committed image" }], next_cursor: null });
  const artifact = { ...record, id: "artifact-1", title: "Committed image", links: [], body: {
    kind: "artifact" as const, media_type: "image/png", byte_size: 900000, digest: null,
    backings: [{ kind: "git_file", repository: "repo-identity", commit: "a".repeat(40), path: "images/example.png", object_format: "sha1", entry_mode: "100644", blob_oid: null }],
  } };
  const get = vi.fn(async () => artifact);
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list, get, why: vi.fn() }, vi.fn());
  pane.activate("project-1");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("n");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("2 records loaded"));
  expect(list).toHaveBeenLastCalledWith("project-1", { after: "older", limit: 24 }, expect.any(AbortSignal));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("image/png"));
  pane.handleInput("\u001b[6~");
  expect(pane.render(100).join("\n")).toContain("images/example.png");
  expect(pane.render(100).join("\n")).toContain("a".repeat(40));
  pane.dispose();
});

it("applies repository/model/search/kind filters through bounded reads", async () => {
  const list = vi.fn(async () => ({ records: [], next_cursor: null }));
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list, get: vi.fn(), why: vi.fn() }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  for (const [key, text] of [["o", "nexus"], ["m", "model-a"], ["/", "regression"]]) {
    pane.handleInput(key); pane.handleInput(text); pane.handleInput("\r");
    await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  }
  pane.handleInput("t");
  await vi.waitFor(() => expect(list).toHaveBeenLastCalledWith(null,
    { repository: "nexus", model: "model-a", search: "regression", kind: "change", limit: 24 }, expect.any(AbortSignal)));
  pane.handleInput("c");
  await vi.waitFor(() => expect(list).toHaveBeenLastCalledWith(null, { limit: 24 }, expect.any(AbortSignal)));
  pane.dispose();
});

it("labels display truncation without altering the retained response", async () => {
  const long = { ...record, body: { kind: "change" as const, motivation: "evidence ".repeat(500) } };
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list: async () => ({ records: [header], next_cursor: null }), get: async () => long, why: vi.fn() }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  for (let i = 0; i < 5; i++) pane.handleInput("\u001b[6~");
  expect(pane.render(100).join("\n")).toContain("display preview truncated");
  expect(long.body.motivation.length).toBe(4500);
  pane.dispose();
});

it("cancels an unfinished detail read when returning to the retained list", async () => {
  const get = vi.fn((_project: string | null, _id: string, _signal?: AbortSignal) => new Promise<never>(() => {}));
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list: async () => ({ records: [header], next_cursor: null }), get, why: vi.fn() }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  expect(pane.statusLine()).toContain("loading");
  pane.handleInput("\u001b");
  expect(get.mock.calls[0][2]?.aborted).toBe(true);
  expect(pane.statusLine()).not.toContain("loading");
  expect(pane.render(100).join("\n")).toContain(header.title);
  pane.dispose();
});

it("rejects a detailed response for a different record identity", async () => {
  const api = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record })) }, "/repo");
  await expect(api.get(null, "different-id")).rejects.toThrow("unexpected response");
});

it("rejects missing typed links instead of guessing references from record contents", async () => {
  const { links: _links, ...withoutLinks } = record;
  const api = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record: withoutLinks })) }, "/repo");
  await expect(api.get(null, record.id)).rejects.toThrow("unexpected response");
});

it("displays imported source receipt separately from the bound receipt and rejects malformed source", async () => {
  const imported = { ...record, source: { project_id: "original-project", recorded_at: "2026-10-20T10:00:00Z",
    provenance: { agent: "original-author", session_id: "original-session", model_id: null, config_hash: "original-config" } } };
  const api = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record: imported })) }, "/repo");
  const decoded = await api.get(null, imported.id);
  const lines = evalDetailLines(decoded!).join("\n");
  expect(lines).toContain("original source receipt");
  expect(lines).toContain("original-project");
  expect(lines).toContain("original-author");
  expect(lines).toContain("2026-10-20T10:00:00Z");
  expect(lines).toContain(`Recorded: ${record.recorded_at}`);
  const invalid = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record: { ...imported, source: { ...imported.source, recorded_at: 123 } } })) }, "/repo");
  await expect(invalid.get(null, imported.id)).rejects.toThrow("unexpected response");
});

it("shows unresolved lifecycle frontiers and partial history without choosing a current state", async () => {
  const lifecycle = { initial_state: "planned", truncated: true, frontiers: [
    { id: "completed-event", state: "completed", occurred_at: "2026-10-22T12:02:00Z", recorded_at: "2026-10-22T12:03:00Z" },
    { id: "failed-event", state: "failed", occurred_at: "2026-10-22T12:02:00Z", recorded_at: "2026-10-22T12:04:00Z" },
  ] };
  const run = { ...record, id: "run-1", lifecycle, body: { kind: "run" as const, state: "planned" } };
  const api = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record: run })) }, "/repo");
  const decoded = await api.get(null, run.id);
  const lines = evalDetailLines(decoded!).join("\n");
  expect(lines).toContain("initial planned");
  expect(lines).toContain("Unresolved lifecycle fork");
  expect(lines).toContain("Lifecycle history is partial");
  expect(lines).toContain("completed-event");
  expect(lines).toContain("failed-event");
  const invalid = new NexusEvalsApi({ call: vi.fn(async () => ({ ok: true, action: "detailed", record: {
    ...run, lifecycle: { ...lifecycle, frontiers: [{ ...lifecycle.frontiers[0], state: "exact" }] },
  } })) }, "/repo");
  await expect(invalid.get(null, run.id)).rejects.toThrow("unexpected response");
});

it("includes occurrence time in history rows before opening a record", async () => {
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme,
    { list: async () => ({ records: [header], next_cursor: null }), get: vi.fn(), why: vi.fn() }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(pane.render(100).join("\n")).toContain("2026-10-22 12:00Z");
  pane.dispose();
});

it("accepts metric definitions and measurements without parsing exact value text", async () => {
  const headers = ["metric", "measurement"].map(kind => ({ ...header, id: kind, kind }));
  const call = vi.fn(async () => ({ ok: true, action: "listed", records: headers, next_cursor: null }));
  const api = new NexusEvalsApi({ call }, "/repo");
  expect((await api.list(null, {})).records.map(item => item.kind)).toEqual(["metric", "measurement"]);
  for (const text of ["900719925474099312345678901234567890", "0.123456789012345678901234567890", '{"bins":[1,2],"note":null,"valid":false}']) {
    const measured = { ...record, body: { kind: "measurement" as const, metric_id: "metric-1", value: { storage: "inline", text } } };
    expect(evalDetailLines(measured).join("\n")).toContain(text);
  }
});

it("preserves literal null, false, and zero in structured evidence previews", () => {
  const values = { ...record, body: { kind: "change" as const, recorded_value: null, valid: false, count: 0 } };
  const lines = evalDetailLines(values).join("\n");
  expect(lines).toContain("recorded value: null");
  expect(lines).toContain("valid: false");
  expect(lines).toContain("count: 0");
});

it("follows a labeled metric-definition link from the value to its meaning and formula", async () => {
  const measurement = { ...record, id: "measurement-1", title: "Tail latency", links: [{ relation: "metric_definition", target_id: "metric-1" }],
    body: { kind: "measurement" as const, metric_id: "metric-1", value: { storage: "inline", text: "0.123456789012345678901234567890" } } };
  const metric = { ...record, id: "metric-1", title: "P95 response latency", links: [], body: {
    kind: "metric" as const, identity: "latency", change_id: "change-1", name: "P95 response latency", description: "End-to-end response duration",
    unit: "milliseconds", value_format: "exact decimal text", calculation: { description: "Compute the 95th percentile", formula: "p95(samples)",
      source_artifacts: [], population: "completed requests", aggregation: "nearest-rank", exclusions: "none", missing_data: "report separately" },
  } };
  const get = vi.fn().mockResolvedValueOnce(measurement).mockResolvedValueOnce(metric);
  const pane = new EvalsPane({ requestRender: vi.fn() } as unknown as TUI, theme, {
    list: async () => ({ records: [{ ...header, id: measurement.id, title: measurement.title, kind: "measurement" as const }], next_cursor: null }), get, why: vi.fn(),
  }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.render(120).join("\n")).toContain("0.123456789012345678901234567890"));
  expect(pane.render(120).join("\n")).toContain("Metric definition: metric-1");
  pane.handleInput("l");
  expect(pane.render(120).join("\n")).toContain("metric_definition → metric-1");
  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.render(120).join("\n")).toContain("p95(samples)"));
  expect(pane.render(120).join("\n")).toContain("unit: milliseconds");
  expect(get).toHaveBeenLastCalledWith(null, "metric-1", expect.any(AbortSignal));
  pane.dispose();
});
