// @feature observability-ui
// @spec docs/features/observability-ui.md
import { describe, expect, it } from "vitest";
import { localNexusUrl } from "../src/client.ts";

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
});
