// @feature observability-ui
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/usage-analytics.md
import { afterEach, describe, expect, it, vi } from "vitest";
import { fetchUsage, localNexusUrl } from "../src/client.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("local Nexus URL validation", () => {
  it.each([
    "http://127.0.0.1:7337",
    "http://localhost:7337",
    "http://[::1]:7337",
  ])("accepts loopback URL %s", (value) => {
    expect(localNexusUrl(`${value}\n`).toString()).toBe(`${value}/`);
  });

  it.each(["https://localhost:7337", "http://example.com:7337", "not a URL"])(
    "rejects non-local Nexus URL %s",
    (value) => {
      expect(() => localNexusUrl(value)).toThrow();
    },
  );

  it("fetches usage with the selected project scope", async () => {
    const response = {
      ok: true,
      project_id: "project-1",
      window_started_at: "2026-09-27T12:00:00Z",
      window_ended_at: "2026-10-04T12:00:00Z",
      tools: [],
      skills: [],
      capture_health: {},
    };
    const fetchMock = vi.fn(async (_input: URL | RequestInfo) =>
      new Response(JSON.stringify(response)),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      fetchUsage(new URL("http://127.0.0.1:7337"), "project-1"),
    ).resolves.toEqual(response);
    expect(String(fetchMock.mock.calls[0][0])).toBe(
      "http://127.0.0.1:7337/api/v1/usage?project_id=project-1",
    );
  });
});
