// @feature observability-ui
// @feature evals
// @spec docs/features/observability-ui.md
// @spec docs/features/evals.md
use super::HistoryRange;
use crate::coordination::api::EvalAction;
use crate::evals::EvalQuery;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum EvalRead {
    #[serde(rename = "eval_dashboard")]
    Dashboard {
        group_id: Option<String>,
        #[serde(default)]
        range: HistoryRange,
    },
    #[serde(rename = "eval_list")]
    List {
        #[serde(default)]
        filter: EvalQuery,
    },
    #[serde(rename = "eval_get")]
    Get { id: String },
    #[serde(rename = "eval_why")]
    Why { id: String },
}
impl EvalRead {
    pub(super) fn into_action(self, project_id: Option<String>) -> EvalAction {
        match self {
            Self::Dashboard { group_id, range } => EvalAction::Dashboard {
                project_id,
                group_id,
                occurred_after: Some(range.cutoff()),
            },
            Self::List { filter } => EvalAction::List {
                project_id,
                kind: filter.kind,
                repository: filter.repository,
                model: filter.model,
                id_prefix: filter.id_prefix,
                search: filter.search,
                after: filter.after,
                limit: filter.limit,
            },
            Self::Get { id } => EvalAction::Get { project_id, id },
            Self::Why { id } => EvalAction::Why {
                project_id,
                id,
                limit: Some(64),
            },
        }
    }
}
