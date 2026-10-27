// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
// @boundary work-activity-kind: known recorded kind families receive display styles; custom values stay generic.
import type { DashboardColor } from "./theme.ts";

interface ActivityStyle { label: string; color: DashboardColor }
const STYLES: Record<string, ActivityStyle> = {
  agent: { label: "Subagent", color: "accent" }, subagent: { label: "Subagent", color: "accent" },
  test: { label: "Test", color: "success" }, script: { label: "Script", color: "warning" },
  review: { label: "Review", color: "error" }, tool: { label: "Tool", color: "muted" },
  command: { label: "Command", color: "muted" }, session: { label: "Session", color: "dim" },
};
/** Display only the recorded kind family; titles and executable names cannot establish task purpose. */
export function activityStyle(kind: string): ActivityStyle {
  if (kind === "work.session" || kind === "host.session") return STYLES.session!;
  const family = kind.toLowerCase().split(".")[0]!;
  return Object.hasOwn(STYLES, family) ? STYLES[family]! : { label: "Other", color: "dim" };
}
