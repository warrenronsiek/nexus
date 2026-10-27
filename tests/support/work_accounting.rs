// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: source boundaries decode through the closed work API.
use nexus::persistence::Store;
use nexus::work_history::WorkActor;
use serde_json::{json, Value};
#[path = "work_record.rs"]
mod work_record;
pub fn command(store: &mut Store, action: Value) -> Value {
    work_record::command(
        store,
        "accounting",
        &WorkActor {
            agent: "fixture".into(),
            session_id: "host".into(),
            model_id: None,
            config_hash: "fixture".into(),
        },
        action,
    )
}
pub fn at(id: &str, scope: &str, subject: &str, second: i64, body: Value) -> Value {
    let mut observation = work_record::event(id, scope, subject, body);
    observation["occurred_at"] = json!((chrono::DateTime::parse_from_rfc3339(
        "2026-10-27T12:00:00Z"
    )
    .unwrap()
        + chrono::Duration::seconds(second))
    .to_rfc3339());
    observation
}
pub fn interval(
    id: &str,
    parent: Option<&str>,
    executor: Option<&str>,
    start: i64,
    end: i64,
) -> Vec<Value> {
    vec![
        at(
            &format!("{id}-declared"),
            "activity",
            id,
            start,
            json!({"event":"activity_declared","name":id,"kind":"caller.task","work_session_id":"root","parent_activity_id":parent,"executor":executor,"context":{}}),
        ),
        at(
            &format!("{id}-start"),
            "activity",
            id,
            start,
            json!({"event":"started"}),
        ),
        at(
            &format!("{id}-finish"),
            "activity",
            id,
            end,
            json!({"event":"finished","outcome":"succeeded","elapsed":{"nanoseconds":((end-start)*1_000_000_000).to_string(),"method":"monotonic","clock_id":"fixture"}}),
        ),
    ]
}
