// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
import { expect, it } from "vitest";
import { activityStyle } from "../src/work-activity-style.ts";

it("distinguishes recorded activity families without inferring purpose from names", () => {
  expect(activityStyle("agent.task")).toEqual({ label: "Subagent", color: "accent" });
  expect(activityStyle("test.rust")).toEqual({ label: "Test", color: "success" });
  expect(activityStyle("script")).toEqual({ label: "Script", color: "warning" });
  expect(activityStyle("review.architecture").label).toBe("Review");
  expect(activityStyle("tool.call").label).toBe("Tool");
  expect(activityStyle("work.session").label).toBe("Session");
  expect(activityStyle("host.session").label).toBe("Session");
  expect(activityStyle("command").label).toBe("Command");
  for (const custom of ["contest", "my-test", "python", "bash", "custom", "constructor", "toString", "__proto__"]) expect(activityStyle(custom).label).toBe("Other");
});
