// @feature work-history
// @feature observability-ui
// @spec docs/features/work-history.md
// @spec docs/features/observability-ui.md
// @entrypoint WorkPane
import { defineHistoryPane, type HistoryPane } from "./history-pane.ts";
import { initialWorkState, resetWorkList } from "./work-state.ts";
import { WorkRequests } from "./work-requests.ts";
import { handleWorkInput, refreshWork } from "./work-navigation.ts";
import { renderWork } from "./work-view.ts";

export type WorkPane = HistoryPane;
export const WorkPane = defineHistoryPane({
  title: "Work history", initial: initialWorkState, requests: WorkRequests,
  reset: resetWorkList, refresh: refreshWork, input: handleWorkInput, render: renderWork,
});
