// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: exercise the typed public work-record lookup contract using closed fixture envelopes.
#![cfg(feature = "sqlite")]
use nexus::persistence::Store;
use nexus::work_history::WorkActor;
use serde_json::json;
#[path = "support/work_record.rs"]
mod work_record;

#[test]
fn exact_observation_lookup_is_bounded_and_project_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("work.db")).unwrap();
    let actor = WorkActor {
        agent: "test".into(),
        session_id: "host".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let observation = work_record::event(
        "stable-callback",
        "activity",
        "tool",
        json!({"event":"started"}),
    );
    work_record::command(
        &mut store,
        "project",
        &actor,
        json!({"action":"record", "observations":[observation.clone()]}),
    );
    let action =
        serde_json::from_value(json!({"action":"observation","id":"stable-callback"})).unwrap();
    let found =
        serde_json::to_value(store.work_command("project", &actor, &action).unwrap()).unwrap();
    let accepted: nexus::work_history::WorkObservation =
        serde_json::from_value(found["observation"].clone()).unwrap();
    assert_eq!(accepted.input, serde_json::from_value(observation).unwrap());
    let isolated =
        serde_json::to_value(store.work_command("other", &actor, &action).unwrap()).unwrap();
    assert!(isolated["observation"].is_null());
}
