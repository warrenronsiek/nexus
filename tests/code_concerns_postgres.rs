// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @boundary postgres-concerns: use the explicit disposable integration service only.
#![cfg(feature = "postgres")]
#[path = "support/concern_input.rs"]
mod input;
use input::*;
use nexus::{
    code_concerns::{ConcernAction, ConcernActor},
    persistence::Store,
};
use serde_json::{json, Value};

fn call(store: &mut Store, project: &str, action: Value) -> Value {
    let actor = ConcernActor {
        agent: "pg-agent".into(),
        session_id: "pg-host".into(),
        model_id: None,
        config_hash: "pg-config".into(),
    };
    let action: ConcernAction = serde_json::from_value(action).unwrap();
    serde_json::to_value(store.concern_command(project, &actor, &action).unwrap()).unwrap()
}

#[test]
fn postgres_concerns_keep_project_identity_source_dedup_and_reference_preserving_imports() {
    let url =
        std::env::var("NEXUS_TEST_DATABASE_URL").expect("PostgreSQL integration URL required");
    let mut store = Store::connect(&url).unwrap();
    let project = format!("concerns-pg-{}", uuid::Uuid::new_v4());
    let other = format!("concerns-import-{}", uuid::Uuid::new_v4());
    let first = report("first", "router-bug", "one", "router");
    let saved = call(
        &mut store,
        &project,
        json!({"action":"record","observations":[first.clone()]}),
    );
    let retry = call(
        &mut store,
        &project,
        json!({"action":"record","observations":[first]}),
    );
    assert_eq!(
        saved["receipts"][0]["recorded_at"],
        retry["receipts"][0]["recorded_at"]
    );
    call(
        &mut store,
        &project,
        json!({"action":"record","observations":[observation("second","router-bug",json!({"event":"encountered","encounter":encounter("one","router")}))]}),
    );
    let dashboard = call(&mut store, &project, json!({"action":"dashboard"}));
    assert_eq!(dashboard["counts"]["raw_observations"], 2);
    assert_eq!(dashboard["counts"]["known_source_occurrences"], 1);
    assert_eq!(dashboard["areas"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(
            &mut store,
            &other,
            json!({"action":"get","concern_id":"router-bug"})
        )["concern"],
        Value::Null
    );
    let exported = call(&mut store, &project, json!({"action":"export"}));
    call(
        &mut store,
        &other,
        json!({"action":"import","observations":exported["observations"]}),
    );
    let imported = call(
        &mut store,
        &other,
        json!({"action":"get","concern_id":"router-bug"}),
    );
    assert_eq!(
        imported["concern"]["summary"]["counts"]["known_source_occurrences"],
        1
    );
    assert_eq!(
        imported["concern"]["history"][0]["source"]["project_id"],
        project
    );
}
