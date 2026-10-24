// @feature work-history
// @spec docs/features/work-history.md
// @feature observability-ui
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";
import { NexusWorkApi, type WorkSummary } from "../src/work.ts";
import { WorkPane } from "../src/work-pane.ts";

const context = { host_session_id: "host-1", turn_id: null, agent_instance_id: "agent-a", model_id: null,
  repository: "nexus", worktree: "/repo", environment: null, revision: null, external: [], labels: {} };
const session: WorkSummary = { subject: { scope: "session", id: "work-1" }, title: "Improve fallback", kind: "work.session",
  work_session_ids: ["work-1"], parent_activity_id: null, has_children: true, state: "running", outcome: null,
  started_at: "2026-10-24T12:00:00Z", finished_at: null, elapsed: null, conflicts: [], gaps: ["finish_missing"], context };
function activity(id: string, parent: string | null, title = id): WorkSummary {
  return { ...session, subject: { scope: "activity", id }, title, kind: "test", parent_activity_id: parent,
    has_children: id === "agent-a" || id === "agent-b", state: "finished", outcome: "succeeded", gaps: [],
    started_at: "2026-10-24T12:00:01Z", finished_at: "2026-10-24T12:00:03Z",
    elapsed: { nanoseconds: "2000000000", method: "monotonic", clock_id: "clock-1" } };
}
const theme = { fg: (_color: string, text: string) => text, bold: (text: string) => text };
const tui = () => ({ requestRender: vi.fn() }) as unknown as TUI;
function screen(pane: WorkPane, width = 120): string { return pane.render(width).join("\n"); }
const sessionPage = async () => ({ items: [session], next_cursor: null });
const envelope = (items: WorkSummary[]) => ({ ok: true, response: "page", items, next_cursor: null });
const timeline = (items: WorkSummary[], parent: string | null = null) => ({ work_session_id: "work-1", parent_activity_id: parent,
  items, next_cursor: null, unresolved_links: [], truncated: false });
const detail = (summary = activity("test-1", "agent-b")) => ({ summary, observations: [], links: [], truncated: false });

async function openWorkSession(pane: WorkPane, project: string | null = null): Promise<void> {
  pane.activate(project); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
}
function press(pane: WorkPane, ...keys: string[]): void { keys.forEach(key => pane.handleInput(key)); }

it("reads bounded work-session pages and rejects malformed and misidentified projections", async () => {
  const call = vi.fn().mockResolvedValueOnce(envelope([session]))
    .mockResolvedValueOnce({ ok: true, response: "detail", record: detail() })
    .mockResolvedValueOnce({ ok: true, response: "timeline", ...timeline([activity("agent-a", null)]) });
  const api = new NexusWorkApi({ call }, "/repo");
  const query = { repository: "nexus", outcome: "failed" as const, limit: 24 };
  expect((await api.list("project-1", query)).items[0].title).toBe("Improve fallback");
  expect(call.mock.calls[0]).toEqual(["nexus_work", { action: "list", query, project_id: "project-1" }, "/repo", undefined]);
  expect((await api.get(null, { scope: "activity", id: "test-1" }))?.summary.subject.id).toBe("test-1");
  expect((await api.timeline(null, "work-1", null, {})).items[0].subject.id).toBe("agent-a");
  const invalid = new NexusWorkApi({ call: vi.fn(async () => envelope([{ ...session, outcome: "approved" } as unknown as WorkSummary])) }, "/repo");
  await expect(invalid.list(null, {})).rejects.toThrow("unexpected response");
  const wrong = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "detail", record: detail() })) }, "/repo");
  await expect(wrong.get(null, { scope: "activity", id: "different" })).rejects.toThrow("unexpected response");
});

it("opens descendants and repeated attempts in an expandable relative timeline", async () => {
  const roots = [activity("agent-a", null, "Implementation agent")];
  const children = [activity("test-1", "agent-a", "Test run 1"), activity("test-2", "agent-a", "Test run 2"), activity("agent-b", "agent-a", "Delegated reviewer")];
  const load = vi.fn(async (_project, _session, parent) => timeline(parent === null ? roots : parent === "agent-a" ? children : [activity("grandchild", "agent-b", "Grandchild test")], parent));
  const get = vi.fn(async (_project, subject) => detail(activity(subject.id, "agent-b", "Grandchild test")));
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: load, get, activities: vi.fn(), events: vi.fn() }, vi.fn());
  await openWorkSession(pane, "project-1");
  expect(screen(pane)).toContain("Implementation agent");
  pane.handleInput("\u001b[C");
  await vi.waitFor(() => expect(screen(pane, 120)).toContain("Test run 2"));
  press(pane, "\u001b[B", "\u001b[B", "\u001b[B", "\u001b[C");
  await vi.waitFor(() => expect(screen(pane, 120)).toContain("Grandchild test"));
  expect(load).toHaveBeenLastCalledWith("project-1", "work-1", "agent-b", { limit: 24 }, expect.any(AbortSignal));
  pane.handleInput("t");
  expect(screen(pane, 120)).toContain("Relative timeline");
  expect(screen(pane, 120)).toContain("+1.000s");
  press(pane, "\u001b[B", "\r");
  await vi.waitFor(() => expect(get).toHaveBeenCalledWith("project-1", { scope: "activity", id: "grandchild" }, expect.any(AbortSignal)));
  expect(screen(pane, 120)).toContain("monotonic");
  for (const width of [20, 40, 80]) expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
  pane.dispose();
});

it("cancels stale scope reads and retains prior data on failed refresh", async () => {
  let complete!: (value: { items: WorkSummary[]; next_cursor: null }) => void;
  const delayed = new Promise<{ items: WorkSummary[]; next_cursor: null }>(resolve => { complete = resolve; });
  const list = vi.fn().mockReturnValueOnce(delayed).mockResolvedValueOnce({ items: [{ ...session, title: "New scope" }], next_cursor: null }).mockRejectedValue(new Error("offline"));
  const pane = new WorkPane(tui(), theme, { list, timeline: vi.fn(), get: vi.fn(), activities: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate("old"); const signal = list.mock.calls[0][2] as AbortSignal;
  pane.activate("new"); expect(signal.aborted).toBe(true);
  await vi.waitFor(() => expect(screen(pane, 100)).toContain("New scope"));
  complete({ items: [session], next_cursor: null }); await delayed;
  expect(screen(pane, 100)).not.toContain("Improve fallback");
  pane.refresh(); await vi.waitFor(() => expect(pane.statusLine()).toContain("stale"));
  expect(screen(pane, 100)).toContain("New scope"); pane.dispose();
});

it("refreshes expanded branches without presenting old descendants as current", async () => {
  let title = "Previous nested test";
  const load = vi.fn(async (_project, _session, parent) => timeline(parent === null ? [activity("agent-a", null, "Agent")] : [activity("test-1", "agent-a", title)], parent));
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: load, get: async () => detail(session), activities: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(screen(pane, 120)).toContain("Agent"));
  pane.handleInput("\u001b[C"); await vi.waitFor(() => expect(screen(pane, 120)).toContain(title));
  title = "Refreshed nested test"; pane.refresh();
  await vi.waitFor(() => expect(screen(pane, 120)).toContain(title));
  expect(screen(pane, 120)).not.toContain("Previous nested test"); pane.dispose();
});

it("puts parallel siblings on one time axis without adding their inclusive durations", async () => {
  const parallel = [activity("review-a", null, "Architecture review"), activity("review-b", null, "QA review")];
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: async () => timeline(parallel), get: vi.fn(), activities: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(screen(pane, 180)).toContain("QA review"));
  pane.handleInput("t");
  const rendered = pane.render(180);
  expect(rendered.join("\n")).toContain("Time axis");
  const bars = rendered.filter(line => /Architecture review|QA review/u.test(line)).map(line => line.match(/\[([ =?]+)\]/u)?.[0]);
  expect(bars[0]).toBeDefined(); expect(bars[0]).toBe(bars[1]);
  expect(rendered.join("\n")).not.toContain("4.000000000s"); pane.dispose();
});

it("pages independent child branches, preserves repeated IDs only once, and exposes partial links", async () => {
  const load = vi.fn().mockResolvedValueOnce(timeline([activity("agent-a", null, "Agent")]))
    .mockResolvedValueOnce({ ...timeline([activity("run-1", "agent-a", "Run one")], "agent-a"), next_cursor: "child-next", truncated: true,
      unresolved_links: [{ event_id: "link-1", source: { scope: "activity", id: "agent-a" }, relation: "depends_on", target: { kind: "activity", id: "missing" }, state: "unresolved" }] })
    .mockResolvedValueOnce(timeline([activity("run-1", "agent-a", "Run one"), activity("run-2", "agent-a", "Run two")], "agent-a"));
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: load, get: vi.fn(), activities: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(screen(pane, 180)).toContain("Agent"));
  pane.handleInput(" "); await vi.waitFor(() => expect(screen(pane, 180)).toContain("Run one"));
  expect(screen(pane, 180)).toContain("Timeline is partial");
  expect(screen(pane, 180)).toContain("activity:missing");
  pane.handleInput("n"); await vi.waitFor(() => expect(screen(pane, 180)).toContain("Run two"));
  expect(load).toHaveBeenLastCalledWith(null, "work-1", "agent-a", { after: "child-next", limit: 24 }, expect.any(AbortSignal));
  expect(screen(pane, 180).match(/Run one/gu)).toHaveLength(1); pane.dispose();
});

it("follows a shared canonical activity link and preserves source receipts and capture gaps", async () => {
  const { observation, sessionDetail, sharedDetail } = sharedFixture();
  const get = vi.fn(async (_project, subject) => subject.id === "work-1" ? sessionDetail : sharedDetail);
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: async () => timeline([]), get, activities: vi.fn(), events: async () => ({ observations: [observation], next_cursor: null }) }, vi.fn());
  await openWorkSession(pane);
  pane.handleInput("s"); await vi.waitFor(() => expect(screen(pane, 160)).toContain("1 recorded links"));
  pane.handleInput("l"); await vi.waitFor(() => expect(screen(pane, 160)).toContain("member_of → activity:release-1"));
  expect(get).toHaveBeenCalledTimes(1);
  pane.handleInput("\r"); await vi.waitFor(() => expect(screen(pane, 160)).toContain("Shared release"));
  expect(screen(pane, 160)).toContain("source_disagreement");
  expect(screen(pane, 160)).toContain("finish_conflict");
  pane.handleInput("e"); await vi.waitFor(() => expect(screen(pane, 160)).toContain("Immutable observations"));
  expect(screen(pane, 160)).toContain("Original receipt: original");
  const rendered = screen(pane, 160);
  expect(rendered).toContain("90071992547409931234567890");
  expect(rendered).toContain("null_value: null"); expect(rendered).toContain("valid: false");
  expect(rendered).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u); pane.dispose();
});

it("applies all root filters through bounded queries and cancels unfinished navigation", async () => {
  const list = vi.fn(sessionPage);
  const activities = vi.fn(async () => ({ items: [activity("test-search", null)], next_cursor: null }));
  const load = vi.fn((_project, _session, _parent, _query, _signal) => new Promise<never>(() => {}));
  const pane = new WorkPane(tui(), theme, { list, timeline: load, get: vi.fn(), activities, events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  for (const [key, value] of [["o", "nexus"], ["h", "host-1"], ["k", "test"], ["v", "beta"], ["/", "router"], ["a", "2026-10-20T00:00:00Z"], ["b", "2026-10-24T23:59:59Z"]]) {
    press(pane, key, value, "\r"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  }
  pane.handleInput("u"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(activities).toHaveBeenLastCalledWith(null, { repository: "nexus", host_session_id: "host-1", kind: "test", environment: "beta", search: "router", occurred_after: "2026-10-20T00:00:00Z", occurred_before: "2026-10-24T23:59:59Z", outcome: "succeeded", limit: 24 }, expect.any(AbortSignal));
  pane.handleInput("c"); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  expect(activities).toHaveBeenLastCalledWith(null, { limit: 24 }, expect.any(AbortSignal));
  pane.dispose();
});

it("keeps unresolved orphan roots visible while rejecting a wrong nested parent", async () => {
  const orphan = { ...activity("orphan", "missing-parent", "Unresolved child"), gaps: ["unresolved_link"] };
  const api = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "timeline", ...timeline([orphan]) })) }, "/repo");
  expect((await api.timeline(null, "work-1", null, {})).items[0].parent_activity_id).toBe("missing-parent");
  const wrong = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "timeline", ...timeline([orphan], "different-parent") })) }, "/repo");
  await expect(wrong.timeline(null, "work-1", "different-parent", {})).rejects.toThrow("unexpected response");
});

it("cancels an unfinished branch expansion when the user collapses it", async () => {
  let complete!: (value: ReturnType<typeof timeline>) => void;
  const delayed = new Promise<ReturnType<typeof timeline>>(resolve => { complete = resolve; });
  const load = vi.fn().mockResolvedValueOnce(timeline([activity("agent-a", null, "Agent")])).mockReturnValueOnce(delayed);
  const pane = new WorkPane(tui(), theme, { list: sessionPage, timeline: load, get: vi.fn(), activities: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.handleInput("\r"); await vi.waitFor(() => expect(screen(pane, 120)).toContain("Agent"));
  pane.handleInput("\u001b[C"); const signal = load.mock.calls[1][4] as AbortSignal;
  pane.handleInput("\u001b[D"); expect(signal.aborted).toBe(true);
  complete(timeline([activity("late-test", "agent-a", "Late child")], "agent-a")); await delayed;
  expect(screen(pane, 120)).not.toContain("Late child"); pane.dispose();
});

function sharedFixture() {
  const shared = activity("release-1", null, "Shared release");
  const observation = { id: "finished-1", subject: shared.subject, occurred_at: "2026-10-24T12:00:03Z", recorded_at: "2026-10-24T12:01:00Z",
    project_id: "project-1", method: "source_import", producer: { name: "ci", version: "1", instance_id: "runner-1" },
    provenance: { agent: "importer", session_id: "host-1", model_id: null, config_hash: "config" },
    source: { project_id: "original", recorded_at: "2026-10-24T12:00:04Z", provenance: { agent: "source-ci", session_id: "ci-1", model_id: null, config_hash: "original-config" } },
    correction: null, body: { event: "finished", assessment: "clean\u001b]52;c;injection\u0007", value: "90071992547409931234567890", null_value: null, valid: false } };
  const sessionDetail = { ...detail(session), links: [{ event_id: "shared-link", source: session.subject, relation: "member_of", target: { kind: "activity" as const, id: shared.subject.id }, state: "resolved" }] };
  const sharedDetail = { ...detail({ ...shared, gaps: ["source_disagreement"], conflicts: ["finish_conflict"] }), observations: [observation] };
  return { shared, observation, sessionDetail, sharedDetail };
}

it("decodes original source receipts independently of the importer", async () => {
  const { shared, sharedDetail } = sharedFixture();
  const api = new NexusWorkApi({ call: vi.fn(async () => ({ ok: true, response: "detail", record: sharedDetail })) }, "/repo");
  const decoded = await api.get(null, shared.subject);
  expect(decoded?.observations[0].source?.provenance.agent).toBe("source-ci");
});

it("routes activity-kind searches to activity pages and follows their work-session hierarchy", async () => {
  const child = activity("test-1", null, "Searched test attempt");
  const activities = vi.fn(async () => ({ items: [child], next_cursor: null }));
  const list = vi.fn(sessionPage);
  const get = vi.fn(async (_project, subject) => detail(subject.scope === "session" ? session : child));
  const pane = new WorkPane(tui(), theme, { list, activities, get, timeline: async () => timeline([child]), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  press(pane, "k", "test", "\r");
  await vi.waitFor(() => expect(activities).toHaveBeenCalledWith(null, { kind: "test", limit: 24 }, expect.any(AbortSignal)));
  expect(list).toHaveBeenCalledTimes(1);
  await vi.waitFor(() => expect(screen(pane)).toContain("Searched test attempt"));
  expect(screen(pane)).toContain("Activity search");
  press(pane, "\r"); await vi.waitFor(() => expect(screen(pane)).toContain("activity:test-1"));
  press(pane, "w"); expect(screen(pane)).toContain("Related work sessions");
  press(pane, "\r"); await vi.waitFor(() => expect(screen(pane)).toContain("Activity hierarchy"));
  expect(get).toHaveBeenLastCalledWith(null, { scope: "session", id: "work-1" }, expect.any(AbortSignal));
  pane.dispose();
});


it("sends typed activity-search queries and rejects session rows in that response", async () => {
  const query = { kind: "test", environment: "beta", limit: 24 };
  const call = vi.fn(async () => envelope([activity("test-1", null)]));
  const api = new NexusWorkApi({ call }, "/repo");
  expect((await api.activities("project-1", query)).items[0].subject.scope).toBe("activity");
  expect(call).toHaveBeenCalledWith("nexus_work", { action: "activities", query, project_id: "project-1" }, "/repo", undefined);
  const wrong = new NexusWorkApi({ call: vi.fn(async () => envelope([session])) }, "/repo");
  await expect(wrong.activities(null, query)).rejects.toThrow("unexpected response");
});

it("keeps unlinked activities readable and discloses missing work-session membership", async () => {
  const unlinked = { ...activity("unlinked", null, "Unlinked captured command"), work_session_ids: [] };
  const pane = new WorkPane(tui(), theme, { list: sessionPage, activities: async () => ({ items: [unlinked], next_cursor: null }),
    get: async () => detail(unlinked), timeline: vi.fn(), events: vi.fn() }, vi.fn());
  pane.activate(null); await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  press(pane, "f"); await vi.waitFor(() => expect(screen(pane)).toContain("Unlinked captured command"));
  press(pane, "\r"); await vi.waitFor(() => expect(screen(pane)).toContain("activity:unlinked"));
  press(pane, "w"); expect(screen(pane)).toContain("No work-session association recorded.");
  pane.dispose();
});
