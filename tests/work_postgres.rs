// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary postgres-work: the integration database is an explicit disposable test service.
#![cfg(feature = "postgres")]
use nexus::persistence::Store;
use nexus::work_history::{WorkAction, WorkActor};
use serde_json::{json, Value};
#[path = "support/work_record.rs"]
mod work_record;
use work_record::event;

fn command(store: &mut Store, project: &str, value: Value) -> Value {
    let action: WorkAction = serde_json::from_value(value).unwrap();
    let actor = WorkActor {
        agent: "pg-test".into(),
        session_id: "pg-host".into(),
        model_id: None,
        config_hash: "pg-config".into(),
    };
    serde_json::to_value(store.work_command(project, &actor, &action).unwrap()).unwrap()
}

#[test]
fn postgres_work_history_preserves_hierarchy_receipts_exact_duration_and_scoped_transfer() {
    let url =
        std::env::var("NEXUS_TEST_DATABASE_URL").expect("PostgreSQL integration URL required");
    let mut store = Store::connect(&url).unwrap();
    let project = format!("work-pg-{}", uuid::Uuid::new_v4());
    let observations = vec![
        event(
            "root",
            "session",
            "work",
            json!({"event":"session_opened","title":"Deliver change","context":{"repository":"router"}}),
        ),
        event(
            "parent",
            "activity",
            "agent",
            json!({"event":"activity_declared","name":"Implementation","kind":"agent.task","work_session_id":"work","context":{}}),
        ),
        event(
            "child",
            "activity",
            "test",
            json!({"event":"activity_declared","name":"Test","kind":"test","parent_activity_id":"agent","context":{}}),
        ),
        event(
            "finished",
            "activity",
            "test",
            json!({"event":"finished","outcome":"succeeded","elapsed":{"nanoseconds":"900719925474099312345","method":"provider"}}),
        ),
    ];
    let first = command(
        &mut store,
        &project,
        json!({"action":"record","observations":observations}),
    );
    let replay = command(
        &mut store,
        &project,
        json!({"action":"record","observations":observations}),
    );
    assert_eq!(first["receipts"], replay["receipts"]);
    assert_eq!(replay["replayed"], 4);
    let children = command(
        &mut store,
        &project,
        json!({"action":"timeline","work_session_id":"work","parent_activity_id":"agent"}),
    );
    assert_eq!(children["items"][0]["subject"]["id"], "test");
    assert_eq!(
        children["items"][0]["elapsed"]["nanoseconds"],
        "900719925474099312345"
    );
    let export = command(&mut store, &project, json!({"action":"export"}));
    let target = format!("{project}-import");
    command(
        &mut store,
        &target,
        json!({"action":"import","observations":export["observations"]}),
    );
    let imported = command(
        &mut store,
        &target,
        json!({"action":"get","subject":{"scope":"activity","id":"test"}}),
    );
    assert_eq!(
        imported["record"]["observations"][0]["source"]["project_id"],
        project
    );
    let hidden = command(
        &mut store,
        "unrelated-work-project",
        json!({"action":"get","subject":{"scope":"activity","id":"test"}}),
    );
    assert!(hidden["record"].is_null());
}
