// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
// @entrypoint ConcernsPane
import { defineHistoryPane, type HistoryPane } from "./history-pane.ts";
import { ConcernsRequests } from "./concerns-requests.ts";
import { handleConcernInput, initialConcernsState, refreshConcerns, resetConcerns } from "./concerns-state.ts";
import { renderConcerns } from "./concerns-view.ts";

export type ConcernsPane = HistoryPane;
export const ConcernsPane = defineHistoryPane({ title: "Code concerns", initial: initialConcernsState, requests: ConcernsRequests,
  reset: resetConcerns, refresh: refreshConcerns, input: handleConcernInput, render: renderConcerns });
