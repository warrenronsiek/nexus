// @feature papercuts
// @feature observability-ui
// @spec docs/features/papercuts.md
// @spec docs/features/observability-ui.md
import { expect, it, vi } from "vitest";
import { NexusPapercutsApi } from "../src/papercuts.ts";
import { PapercutsPane } from "../src/papercuts-pane.ts";
import { visibleWidth, type TUI } from "@earendil-works/pi-tui";

export const issue = {
  id: "cut-1", project_id: "project-1", title: "Slow checks", description: "Build\u001b]52;c;bad\u0007 takes minutes",
  status: "open" as const, upvote_count: 4, last_upvoted_at: "2026-10-19T12:00:00Z",
  created_at: "2026-10-18T12:00:00Z", updated_at: "2026-10-19T12:00:00Z",
  resolution: null, consolidated_into: null,
};

it("reads the selected project and browses sanitized papercut details", async () => {
  const call = vi.fn(async () => ({ ok: true, action: "listed", papercuts: [issue], truncated: true }));
  const pane = new PapercutsPane(
    { requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: text => text },
    new NexusPapercutsApi({ call }, "/repo"), vi.fn(),
  );
  pane.activate("project-1");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Slow checks"));
  expect(call).toHaveBeenCalledWith("nexus_papercuts", { action: "list", project_id: "project-1" }, "/repo", expect.any(AbortSignal));
  expect(pane.render(100).join("\n")).toContain("4 encounters");
  expect(pane.render(100).join("\n")).toContain("more not shown");
  pane.handleInput("\r");
  expect(pane.render(100).join("\n")).toContain("cut-1");
  expect(pane.render(100).join("\n")).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u);
  pane.handleInput("\u001b");
  expect(pane.render(100).join("\n")).toContain("more not shown");
  pane.dispose();
});

it("rejects malformed papercut responses", async () => {
  const api = new NexusPapercutsApi({ call: vi.fn(async () => ({ ok: true, action: "listed", papercuts: [{ ...issue, upvote_count: -1 }], truncated: false })) }, "/repo");
  await expect(api.list(null)).rejects.toThrow("unexpected response");
});

it("aborts an old project read and ignores its late result", async () => {
  let complete!: (value: { papercuts: typeof issue[]; truncated: boolean }) => void;
  const old = new Promise<{ papercuts: typeof issue[]; truncated: boolean }>(resolve => { complete = resolve; });
  const list = vi.fn().mockReturnValueOnce(old).mockResolvedValue({ papercuts: [], truncated: false });
  const pane = new PapercutsPane({ requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: text => text }, { list }, vi.fn());
  pane.activate("old");
  const signal = list.mock.calls[0][1] as AbortSignal;
  pane.deactivate();
  expect(signal.aborted).toBe(true);
  pane.activate("new");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("No open papercuts"));
  complete({ papercuts: [issue], truncated: false });
  await old;
  expect(pane.render(100).join("\n")).not.toContain("Slow checks");
  pane.dispose();
});

it("keeps the last list visible when a refresh fails", async () => {
  const list = vi.fn().mockResolvedValueOnce({ papercuts: [issue], truncated: false }).mockRejectedValue(new Error("offline"));
  const pane = new PapercutsPane({ requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: text => text }, { list }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  pane.refresh();
  await vi.waitFor(() => expect(pane.statusLine()).toContain("stale · offline"));
  expect(pane.render(100).join("\n")).toContain("Slow checks");
  pane.dispose();
});

it("keeps truncated lists and details within narrow terminal widths", async () => {
  const pane = new PapercutsPane({ requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: text => text },
    { list: async () => ({ papercuts: [issue], truncated: true }) }, vi.fn());
  pane.activate(null);
  await vi.waitFor(() => expect(pane.statusLine()).toContain("live"));
  for (const width of [20, 40, 80]) {
    expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
    pane.handleInput("\r");
    expect(pane.render(width).every(line => visibleWidth(line) <= width)).toBe(true);
    pane.handleInput("\u001b");
  }
  pane.dispose();
});
