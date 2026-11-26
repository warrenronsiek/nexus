// @feature observability-ui
// @spec docs/features/observability-ui.md
use crate::agent_traces::{TraceFilter, TraceQuery};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum TraceRead {
    #[serde(rename = "trace_list")]
    List {
        #[serde(default)]
        filter: TraceFilter,
    },
    #[serde(rename = "trace_usage")]
    Usage {
        #[serde(default)]
        filter: TraceFilter,
    },
    #[serde(rename = "trace_calls")]
    Calls {
        #[serde(default)]
        filter: TraceFilter,
    },
    #[serde(rename = "trace_get")]
    Get {
        application_id: String,
        environment: String,
        trace_id: String,
        event_id: Option<String>,
        #[serde(default)]
        include_payload: bool,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    #[serde(rename = "trace_status")]
    Status,
}

impl TraceRead {
    pub(super) fn into_query(self, allow_contents: bool) -> TraceQuery {
        match self {
            Self::List { filter } => TraceQuery::List { filter },
            Self::Usage { filter } => TraceQuery::Usage { filter },
            Self::Calls { filter } => TraceQuery::Calls { filter },
            Self::Get {
                application_id,
                environment,
                trace_id,
                event_id,
                include_payload,
                limit,
                cursor,
            } => TraceQuery::Get {
                application_id,
                environment,
                trace_id,
                event_id,
                include_payload: allow_contents && include_payload,
                limit,
                cursor,
            },
            Self::Status => TraceQuery::Status {
                application_id: None,
                environment: None,
            },
        }
    }
}
