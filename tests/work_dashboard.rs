// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: explicit fixtures decode through the closed WorkAction schema.
#![cfg(feature = "sqlite")]
use nexus::persistence::Store;
use nexus::work_history::WorkActor;
use serde_json::{json, Value};
#[path = "support/work_record.rs"]
mod work_record;
use work_record::event;
fn actor() -> WorkActor {
    WorkActor {
        agent: "test".into(),
        session_id: "host".into(),
        model_id: None,
        config_hash: "test".into(),
    }
}
fn command(store: &mut Store, value: Value) -> Value {
    work_record::command(store, "p", &actor(), value)
}
fn finish(id: &str, scope: &str, subject: &str, seconds: u64) -> Value {
    event(
        id,
        scope,
        subject,
        json!({"event":"finished","outcome":"succeeded","elapsed":{"nanoseconds":(seconds*1_000_000_000).to_string(),"method":"monotonic","clock_id":"process-1"}}),
    )
}
#[test]
fn work_dashboard_summarizes_history_beyond_one_page_and_all_descendants() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    for index in 0..70 {
        let id = format!("session-{index:03}");
        command(
            &mut store,
            json!({"action":"record","observations":[event(&format!("open-{index}"),"session",&id,json!({"event":"session_opened","title":id,"context":{}})),finish(&format!("finish-{index}"),"session",&id,10)]}),
        );
    }
    command(
        &mut store,
        json!({"action":"record","observations":[event("parent","activity","parent",json!({"event":"activity_declared","name":"Agent","kind":"agent","work_session_id":"session-000","context":{}})),event("child","activity","child",json!({"event":"activity_declared","name":"Tests","kind":"test","parent_activity_id":"parent","context":{}})),finish("parent-finish","activity","parent",6),finish("child-finish","activity","child",2)]}),
    );
    let overview = command(&mut store, json!({"action":"dashboard"}));
    assert_eq!(overview["session_duration"]["count"], 70);
    assert_eq!(overview["session_duration"]["known_count"], 70);
    assert_eq!(overview["session_duration"]["total_seconds"], 700.0);
    assert_eq!(overview["daily"][0]["average_seconds"], 10.0);
    let selected = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"session-000"}),
    );
    assert_eq!(selected["activities"].as_array().unwrap().len(), 2);
    assert_eq!(selected["selected_session"]["subject"]["id"], "session-000");
    assert_eq!(selected["activity_kinds"][1]["average_seconds"], 2.0);
    assert_eq!(selected["truncated"], false);
}
#[test]
fn selected_session_keeps_old_descendants_when_other_work_exceeds_project_scan() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("large.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":[event("old-root","session","old",json!({"event":"session_opened","title":"Old","context":{}})),event("old-task","activity","old-task",json!({"event":"activity_declared","name":"Old tests","kind":"test","work_session_id":"old","context":{}}))]}),
    );
    for batch in 0..64 {
        let observations = (0..64)
            .map(|offset| {
                let id = format!("new-{}", batch * 64 + offset);
                event(
                    &id,
                    "session",
                    &id,
                    json!({"event":"session_opened","title":"New unrelated work","context":{}}),
                )
            })
            .collect::<Vec<_>>();
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
    }
    let selected = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"old"}),
    );
    assert_eq!(selected["activities"][0]["subject"]["id"], "old-task");
    assert_eq!(selected["truncated"], true);
}
#[test]
fn selected_session_bounds_pages_of_corrected_away_candidates() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("stale-links.db")).unwrap();
    for batch in 0..129 {
        let mut observations = vec![];
        for offset in 0..32 {
            let index = batch * 32 + offset;
            if index > 4096 {
                break;
            }
            let id = format!("task-{index:04}");
            let declared_id = format!("declared-{index:04}");
            let mut original = event(
                &declared_id,
                "activity",
                &id,
                json!({"event":"activity_declared","name":"Corrected task","kind":"test","work_session_id":"old","context":{}}),
            );
            let mut corrected = original.clone();
            corrected["id"] = json!(format!("corrected-{index:04}"));
            corrected["body"]["work_session_id"] = json!("new");
            corrected["correction"] =
                json!({"replaces":declared_id,"reason":"Caller corrected membership"});
            original["method"] = json!("caller");
            observations.extend([original, corrected]);
        }
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
    }
    let selected = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"old"}),
    );
    assert!(selected["activities"].as_array().unwrap().is_empty());
    assert!(
        selected["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice
                .as_str()
                .unwrap()
                .starts_with("selected_activity_limit")),
        "stale candidate pages need a separately disclosed bounded traversal"
    );
}
