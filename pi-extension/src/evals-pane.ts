// @feature evals
// @feature observability-ui
// @spec docs/features/evals.md
// @spec docs/features/observability-ui.md
// @entrypoint EvalsPane
import { defineHistoryPane, type HistoryPane } from "./history-pane.ts";
import { EvalsRequests } from "./evals-requests.ts";
import { handleEvalInput, initialEvalsState, refreshEvals, resetEvalList } from "./evals-state.ts";
import { renderEvals } from "./evals-view.ts";

export type EvalsPane = HistoryPane;
export const EvalsPane = defineHistoryPane({
  title: "Eval history", initial: initialEvalsState, requests: EvalsRequests,
  reset: resetEvalList, refresh: refreshEvals, input: handleEvalInput, render: renderEvals,
});
