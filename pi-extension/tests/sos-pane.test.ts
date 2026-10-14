// @feature sos
// @feature observability-ui
// @spec docs/features/sos.md
// @spec docs/features/observability-ui.md
import type { TUI } from "@earendil-works/pi-tui";
import { expect, it, vi } from "vitest";
import { SosPane } from "../src/sos-pane.ts";
import type { SosApi, SosIssueDetail, SosList } from "../src/sos.ts";

const maliciousMessage = "pipeline \u001b]52;c;c3RvbGVu\u0007wedged";
const issue = {
  id: "sos-1",
  project_id: "project-1",
  title: "Release pipeline is wedged",
  summary: "Publishing repeatedly exits.",
  status: "open",
  resolution: null,
  created_at: "2026-10-14T12:00:00Z",
  updated_at: "2026-10-14T12:05:00Z",
  closed_at: null,
  first_reported_at: "2026-10-14T12:00:00Z",
  last_reported_at: "2026-10-14T12:05:00Z",
  importance_count: 3,
} as const;
const list: SosList = { ok: true, issues: [issue], untriaged_report_count: 2, truncated: false };
const detail: SosIssueDetail = {
  issue,
  reports: [{
    id: "report-1",
    project_id: "project-1",
    issue_id: "sos-1",
    message: maliciousMessage,
    type: "build_failure",
    environment: "ci-linux",
    version: "1.7.0",
    model: "gpt-6",
    trace: "step=publish\nexit=137",
    metadata: { job: "release" },
    reporter: { agent: "codex", session_id: "session-1", model_id: "gpt-6", config_hash: "hash" },
    created_at: "2026-10-14T12:05:00Z",
  }],
  reports_truncated: true,
  triage_attempts: [{
    id: "triage-1",
    project_id: "project-1",
    report_id: "report-1",
    issue_id: "sos-1",
    outcome: "linked",
    provider: "codex",
    model_id: "gpt-6",
    rationale: "Same publishing failure",
    generated_title: "Release pipeline is wedged",
    generated_summary: "Publishing exits early",
    config_hash: "hash",
    created_at: "2026-10-14T12:06:00Z",
  }],
  triage_attempts_truncated: false,
  lifecycle_events: [],
  lifecycle_events_truncated: false,
};

it("shows aggregate importance first and drills into sanitized source detail", async () => {
  const api: SosApi = {
    list: vi.fn(async () => list),
    get: vi.fn(async () => detail),
  };
  const pane = new SosPane(
    { requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: (text) => text },
    api,
    vi.fn(),
  );

  pane.activate();
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("3 reports"));
  expect(pane.render(100).join("\n")).toContain("2 awaiting triage");

  pane.handleInput("\r");
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Source reports"));
  const rendered = pane.render(100).join("\n");
  expect(rendered).toContain("Same publishing failure");
  expect(rendered).toContain("step=publish");
  expect(rendered).toContain("more source reports not shown");
  expect(rendered).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u);
  pane.dispose();
});
it("sanitizes malformed remote timestamps before rendering the issue list", async () => {
  const api: SosApi = {
    list: vi.fn(async () => ({
      ...list,
      issues: [{ ...issue, last_reported_at: "invalid\u001b]52;c;c3RvbGVu\u0007" }],
    })),
    get: vi.fn(async () => detail),
  };
  const pane = new SosPane(
    { requestRender: vi.fn() } as unknown as TUI,
    { fg: (_color, text) => text, bold: (text) => text },
    api,
    vi.fn(),
  );

  pane.activate();
  await vi.waitFor(() => expect(pane.render(100).join("\n")).toContain("Release pipeline"));

  expect(pane.render(100).join("\n")).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u);
  pane.dispose();
});
