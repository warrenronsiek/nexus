// @feature sos
// @feature observability-ui
// @spec docs/features/sos.md
// @spec docs/features/observability-ui.md
import { describe, expect, it, vi } from "vitest";
import {
  NexusSosApi,
  decodeSosDetail,
  decodeSosList,
  type SosIssueDetail,
  type SosList,
} from "../src/sos.ts";
import type { NexusMcpClient } from "../src/mcp-client.ts";

const issue = {
  id: "sos-1",
  project_id: "project-1",
  title: "Release pipeline is wedged",
  summary: "Publishing repeatedly exits before the image is available.",
  status: "open",
  resolution: null,
  created_at: "2026-10-14T12:00:00Z",
  updated_at: "2026-10-14T12:05:00Z",
  closed_at: null,
  first_reported_at: "2026-10-14T12:00:00Z",
  last_reported_at: "2026-10-14T12:05:00Z",
  importance_count: 3,
} as const;

const report = {
  id: "report-1",
  project_id: "project-1",
  issue_id: "sos-1",
  message: "Release pipeline is wedged",
  type: "build_failure",
  environment: "ci-linux",
  version: "1.7.0",
  model: "gpt-6",
  trace: "step=publish\nexit=137",
  metadata: { job: "release", attempt: 3 },
  reporter: {
    agent: "codex",
    session_id: "session-1",
    model_id: "gpt-6",
    config_hash: "config-hash",
  },
  created_at: "2026-10-14T12:05:00Z",
} as const;

const listResponse = {
  ok: true,
  issues: [issue],
  untriaged_report_count: 2,
  truncated: false,
} satisfies SosList;

const detailResponse = {
  issue,
  reports: [report],
  reports_truncated: false,
  triage_attempts: [],
  triage_attempts_truncated: false,
  lifecycle_events: [],
  lifecycle_events_truncated: false,
} satisfies SosIssueDetail;

describe("SOS MCP client boundary", () => {
  it("strictly decodes aggregate summaries and source-report detail", () => {
    expect(decodeSosList(listResponse)).toEqual(listResponse);
    expect(decodeSosDetail({ ok: true, detail: detailResponse })).toEqual(detailResponse);

    expect(() => decodeSosList({ ...listResponse, issues: [{ ...issue, importance_count: "3" }] }))
      .toThrow("unexpected response");
    expect(() => decodeSosDetail({
      ok: true,
      detail: { ...detailResponse, reports: [{ ...report, metadata: [] }] },
    })).toThrow("unexpected response");
  });

  it("uses one typed SOS tool for read-only list and detail", async () => {
    const call = vi.fn(async (_tool: string, argumentsValue: Record<string, unknown>) => {
      if (argumentsValue.action === "list") return listResponse;
      if (argumentsValue.action === "get") return { ok: true, detail: detailResponse };
      throw new Error("unexpected action");
    });
    const api = new NexusSosApi({ call } as Pick<NexusMcpClient, "call">, "/repo");

    await expect(api.list()).resolves.toEqual(listResponse);
    await expect(api.get("sos-1")).resolves.toEqual(detailResponse);

    expect(call).toHaveBeenNthCalledWith(1, "nexus_sos", { action: "list" }, "/repo", undefined);
    expect(call).toHaveBeenNthCalledWith(2, "nexus_sos", { action: "get", issue_id: "sos-1" }, "/repo", undefined);
    expect(call).toHaveBeenCalledTimes(2);
  });
});
