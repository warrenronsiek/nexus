// @feature runtime
// @feature evals
// @spec docs/features/runtime.md
// @spec docs/features/evals.md
// @boundary generated-json-schema
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_evals",
        "description":"Record and retrieve immutable evaluation history: motivations, repository/model revisions, caller-owned runs, assessments and decisions. Prefer pinned Git artifacts. Nexus does not execute evaluations or graders. list is paginated; get and why expand evidence. Caller identity is supplied by Nexus.",
        "inputSchema":schema()
    })
}

pub(crate) fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(crate::coordination::api::EvalAction))
        .expect("eval schema is serializable")
}
