// @feature work-history
// @spec docs/features/work-history.md
// @boundary fixture-json: callers decode these explicit lifecycle fixture values through the closed work schema.
use serde_json::{json, Value};
pub fn event(id: &str, scope: &str, subject: &str, body: Value) -> Value {
    json!({"id":id,"subject":{"scope":scope,"id":subject},"occurred_at":"2026-10-24T12:00:00Z","producer":{"name":"fixture","version":"1","instance_id":"process-1"},"method":"caller","body":body})
}

pub fn command(
    store: &mut nexus::persistence::Store,
    project: &str,
    actor: &nexus::work_history::WorkActor,
    value: Value,
) -> Value {
    let action = serde_json::from_value(value).unwrap();
    serde_json::to_value(store.work_command(project, actor, &action).unwrap()).unwrap()
}
