// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary generated-json-schema
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_work",
        "description":"Record and explore durable work sessions, nested agent/command/test/review activities, milestones, retries and delivery evidence. Callers execute work. Events are immutable and idempotent; occurrence and receipt times remain distinct. dashboard summarizes qualified durations by kind/day and a selected session's descendants; list pages work sessions by root attributes; activities searches individual invocations including kind/environment/outcome and retains their tree links; timeline pages children; dashboard supplies qualified session/activity duration aggregates, UTC trends, and selected-session descendant intervals with explicit source/display bounds; get discloses details and capture gaps. Caller provenance is bound by Nexus.",
        "inputSchema":schemars::schema_for!(crate::work_history::WorkAction)
    })
}
