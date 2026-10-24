// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary generated-json-schema
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_work",
        "description":"Record and explore durable work sessions, nested agent/command/test/review activities, milestones, retries and delivery evidence. Callers execute work. Events are immutable and idempotent; occurrence and receipt times remain distinct. list pages work sessions by root attributes; activities searches individual invocations including kind/environment/outcome and retains their tree links; timeline pages children; get discloses details and capture gaps. Caller provenance is bound by Nexus.",
        "inputSchema":schemars::schema_for!(crate::work_history::WorkAction)
    })
}
