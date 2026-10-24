// @feature work-history
// @spec docs/features/work-history.md
// @boundary fixture-json: closed work commands are decoded from explicit test fixture values.
#![cfg(feature = "sqlite")]
use nexus::persistence::Store;
use nexus::work_history::{WorkAction, WorkActor};
use serde_json::{json, Value};
#[path = "support/work_record.rs"]
mod work_record;
use work_record::event;

fn actor() -> WorkActor {
    WorkActor {
        agent: "test".into(),
        session_id: "host-session".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}
fn command(store: &mut Store, action: Value) -> Value {
    let action: WorkAction = serde_json::from_value(action).unwrap();
    serde_json::to_value(store.work_command("project", &actor(), &action).unwrap()).unwrap()
}

#[test]
fn work_history_preserves_nested_attempts_exact_timing_and_immutable_replay() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let records = vec![
        event(
            "root",
            "session",
            "work",
            json!({"event":"session_opened","title":"Ship router","context":{}}),
        ),
        event(
            "agent",
            "activity",
            "agent-task",
            json!({"event":"activity_declared","name":"Implementation","kind":"agent.task","work_session_id":"work","context":{}}),
        ),
        event(
            "child",
            "activity",
            "test-1",
            json!({"event":"activity_declared","name":"Tests","kind":"test","parent_activity_id":"agent-task","context":{}}),
        ),
        event(
            "finish",
            "activity",
            "test-1",
            json!({"event":"finished","outcome":"failed","elapsed":{"nanoseconds":"900719925474099312345","method":"monotonic","clock_id":"process-1"}}),
        ),
        event("start", "activity", "test-1", json!({"event":"started"})),
    ];
    let first = command(
        &mut store,
        json!({"action":"record","observations":records}),
    );
    assert_eq!(first["inserted"], 5);
    let replay = command(
        &mut store,
        json!({"action":"record","observations":records}),
    );
    assert_eq!(replay["replayed"], 5);
    assert_eq!(first["receipts"], replay["receipts"]);
    let root = command(
        &mut store,
        json!({"action":"timeline","work_session_id":"work"}),
    );
    assert_eq!(root["items"][0]["subject"]["id"], "agent-task");
    let children = command(
        &mut store,
        json!({"action":"timeline","work_session_id":"work","parent_activity_id":"agent-task"}),
    );
    assert_eq!(children["items"][0]["subject"]["id"], "test-1");
    assert_eq!(
        children["items"][0]["elapsed"]["nanoseconds"],
        "900719925474099312345"
    );
    assert_eq!(children["items"][0]["outcome"], "failed");
    assert!(children["items"][0]["gaps"].as_array().unwrap().is_empty());
    let mut conflict = records[0].clone();
    conflict["body"]["title"] = json!("Changed");
    let result = store.work_command(
        "project",
        &actor(),
        &serde_json::from_value(json!({"action":"record","observations":[conflict]})).unwrap(),
    );
    assert!(result.unwrap_err().to_string().contains("immutable"));
}

#[test]
fn late_links_conflicts_corrections_and_cross_clock_gaps_remain_visible() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let child = event(
        "child",
        "activity",
        "child",
        json!({"event":"activity_declared","name":"Child","kind":"agent.task","parent_activity_id":"late","context":{}}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[child]}),
    );
    let get = json!({"action":"get","subject":{"scope":"activity","id":"child"}});
    let detail = command(&mut store, get.clone());
    assert!(detail["record"]["summary"]["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("unresolved_link")));
    command(
        &mut store,
        json!({"action":"record","observations":[event("late","activity","late",json!({"event":"activity_declared","name":"Late parent","kind":"agent.task","parent_activity_id":"child","work_session_id":"work","context":{}}))]}),
    );
    let detail = command(&mut store, get);
    assert!(detail["record"]["summary"]["conflicts"]
        .as_array()
        .unwrap()
        .contains(&json!("parent_cycle")));
    assert_eq!(
        detail["record"]["summary"]["parent_activity_id"],
        Value::Null
    );
}

#[test]
fn correction_chains_preserve_originals_and_expose_competing_finishes() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let finish = event(
        "finish-a",
        "activity",
        "task",
        json!({"event":"finished","outcome":"failed"}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[finish]}),
    );
    let mut corrected = event(
        "finish-b",
        "activity",
        "task",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    corrected["correction"] =
        json!({"replaces":"finish-a","reason":"Original status was transcribed incorrectly"});
    command(
        &mut store,
        json!({"action":"record","observations":[corrected]}),
    );
    let task = json!({"action":"get","subject":{"scope":"activity","id":"task"}});
    let detail = command(&mut store, task.clone());
    assert_eq!(detail["record"]["summary"]["outcome"], "succeeded");
    assert_eq!(
        detail["record"]["observations"].as_array().unwrap().len(),
        2
    );
    let mut branch = event(
        "finish-c",
        "activity",
        "task",
        json!({"event":"finished","outcome":"canceled"}),
    );
    branch["correction"] = json!({"replaces":"finish-a","reason":"Competing explanation"});
    command(
        &mut store,
        json!({"action":"record","observations":[branch]}),
    );
    let detail = command(&mut store, task);
    assert_eq!(detail["record"]["summary"]["state"], "conflicted");
    assert_eq!(detail["record"]["summary"]["outcome"], Value::Null);
}

#[test]
fn work_pages_freeze_population_and_transfer_retains_orphans_source_and_exact_values() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let inputs = history_fixture();
    command(&mut store, json!({"action":"record","observations":inputs}));
    let first = command(&mut store, json!({"action":"list","query":{"limit":1}}));
    command(
        &mut store,
        json!({"action":"record","observations":[event("event-z","session","z",json!({"event":"session_opened","title":"Z","context":{}}))]}),
    );
    let next = command(
        &mut store,
        json!({"action":"list","query":{"limit":1,"after":first["next_cursor"]}}),
    );
    assert_eq!(next["items"][0]["subject"]["id"], "d");
    assert_eq!(next["next_cursor"], Value::Null);
}

fn history_fixture() -> Vec<Value> {
    vec![
        event(
            "event-b",
            "session",
            "b",
            json!({"event":"session_opened","title":"B","context":{}}),
        ),
        event(
            "event-d",
            "session",
            "d",
            json!({"event":"session_opened","title":"D","context":{}}),
        ),
        event(
            "orphan",
            "activity",
            "orphan",
            json!({"event":"finished","outcome":"failed","elapsed":{"nanoseconds":"900719925474099312345","method":"provider"}}),
        ),
    ]
}

#[test]
fn work_transfer_preserves_orphans_source_and_exact_values() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":history_fixture()}),
    );
    let export = command(&mut store, json!({"action":"export"}));
    let mut target = Store::open(&temporary.path().join("other.db")).unwrap();
    command(
        &mut target,
        json!({"action":"import","observations":export["observations"]}),
    );
    let detail = command(
        &mut target,
        json!({"action":"get","subject":{"scope":"activity","id":"orphan"}}),
    );
    assert_eq!(
        detail["record"]["summary"]["elapsed"]["nanoseconds"],
        "900719925474099312345"
    );
    assert_eq!(
        detail["record"]["observations"][0]["source"]["recorded_at"],
        export["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "orphan")
            .unwrap()["recorded_at"]
    );
    let snapshot = temporary.path().join("snapshot.jsonl");
    store
        .write_streamed_snapshot(std::fs::File::create(&snapshot).unwrap())
        .unwrap();
    let mut restored = Store::open(&temporary.path().join("restored.db")).unwrap();
    restored.import_streamed_snapshot(&snapshot).unwrap();
    let replay = command(&mut restored, json!({"action":"export"}));
    assert_eq!(export["observations"], replay["observations"]);
}

#[test]
fn invalid_corrections_are_typed_rejections_and_late_wrong_subjects_are_conflicts() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let mut correction = event(
        "correction",
        "activity",
        "task",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    correction["correction"] =
        json!({"replaces":"unknown","reason":"Imported correction arrived first"});
    command(
        &mut store,
        json!({"action":"record","observations":[correction]}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[event("unknown","activity","other",json!({"event":"finished","outcome":"failed"}))]}),
    );
    let detail = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert!(detail["record"]["summary"]["conflicts"]
        .as_array()
        .unwrap()
        .contains(&json!("invalid_correction_scope")));
    let mut invalid = event(
        "invalid",
        "activity",
        "third",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    invalid["correction"] = json!({"replaces":"unknown","reason":"Wrong subject"});
    let error = store
        .work_command(
            "project",
            &actor(),
            &serde_json::from_value(json!({"action":"record","observations":[invalid]})).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<nexus::work_history::WorkRejection>(),
        Some(nexus::work_history::WorkRejection::Invalid(_))
    ));
}

#[test]
fn known_parent_cycles_are_rejected_without_losing_prior_observations() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":[event("a-declared","activity","a",json!({"event":"activity_declared","name":"A","kind":"agent.task","context":{}})),event("b-declared","activity","b",json!({"event":"activity_declared","name":"B","kind":"agent.task","context":{}}))]}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[event("parent-a","activity","a",json!({"event":"linked","relation":"parent","target":{"kind":"activity","id":"b"}}))]}),
    );
    let cycle=serde_json::from_value(json!({"action":"record","observations":[event("parent-b","activity","b",json!({"event":"linked","relation":"parent","target":{"kind":"activity","id":"a"}}))]})).unwrap();
    let error = store.work_command("project", &actor(), &cycle).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<nexus::work_history::WorkRejection>(),
        Some(nexus::work_history::WorkRejection::Invalid(_))
    ));
    let b = command(
        &mut store,
        json!({"action":"events","subject":{"scope":"activity","id":"b"}}),
    );
    assert_eq!(b["observations"].as_array().unwrap().len(), 1);
}

#[test]
fn detail_discloses_byte_truncation_without_exceeding_transport_budget() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    for batch in 0..2 {
        let observations=(0..64).map(|offset|event(&format!("link-{:03}",batch*64+offset),"activity","task",json!({"event":"linked","relation":"evidence","target":{"kind":"external","provider":"ci","type":"job","id":"x".repeat(1000),"url":format!("https://example.test/{}","y".repeat(4000))}}))).collect::<Vec<_>>();
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
    }
    let detail = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert!(
        serde_json::to_vec(&detail).unwrap().len() <= 768 * 1024,
        "detail exceeds transport budget"
    );
    assert_eq!(detail["record"]["truncated"], true);
}

fn reported_span(finish_time: &str, finish_clock: &str) -> Value {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let mut finish = event(
        "finish",
        "activity",
        "task",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    finish["occurred_at"] = json!(finish_time);
    finish["producer"]["instance_id"] = json!(finish_clock);
    command(
        &mut store,
        json!({"action":"record","observations":[event("start","activity","task",json!({"event":"started"})),finish]}),
    );
    command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    )["record"]["summary"]
        .clone()
}

#[test]
fn wall_clock_fallback_is_qualified_and_never_subtracts_unrelated_clocks() {
    let same = reported_span("2026-10-24T12:00:02Z", "process-1");
    assert_eq!(same["elapsed"]["nanoseconds"], "2000000000");
    assert_eq!(same["elapsed"]["method"], "wall_clock");
    let other = reported_span("2026-10-24T12:00:02Z", "another-process");
    assert!(other["elapsed"].is_null());
    assert!(other["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("clock_domain_mismatch")));
    let negative = reported_span("2026-10-24T11:59:59Z", "process-1");
    assert!(negative["elapsed"].is_null());
    assert!(negative["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("wall_clock_anomaly")));
}

#[test]
fn activity_search_filters_one_invocation_and_root_lists_reject_activity_kinds() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let invalid = serde_json::from_value(json!({"action":"list","query":{"kind":"test"}})).unwrap();
    assert!(
        store.work_command("project", &actor(), &invalid).is_err(),
        "root kind filters must not silently return no matching work"
    );
    command(
        &mut store,
        json!({"action":"record","observations":[event("root","session","work",json!({"event":"session_opened","title":"Root","context":{}})),event("test","activity","test",json!({"event":"activity_declared","name":"Tests","kind":"test","work_session_id":"work","context":{"repository":"router","environment":"beta"}}))]}),
    );
    let result = command(
        &mut store,
        json!({"action":"activities","query":{"kind":"test","repository":"router","environment":"beta"}}),
    );
    assert_eq!(result["items"][0]["subject"]["id"], "test");
    assert_eq!(result["items"][0]["work_session_ids"], json!(["work"]));
    let wrong = command(
        &mut store,
        json!({"action":"activities","query":{"kind":"test","repository":"different","environment":"beta"}}),
    );
    assert!(wrong["items"].as_array().unwrap().is_empty());
}

#[test]
fn portable_work_import_preserves_previously_accepted_late_relationship_conflicts() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let mut correction = event(
        "correction",
        "activity",
        "task",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    correction["correction"] =
        json!({"replaces":"unknown","reason":"Source reported correction before its target"});
    command(
        &mut store,
        json!({"action":"record","observations":[correction]}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[event("unknown","activity","other",json!({"event":"finished","outcome":"failed"}))]}),
    );
    let export = command(&mut store, json!({"action":"export"}));
    let mut target = Store::open(&temporary.path().join("target.db")).unwrap();
    command(
        &mut target,
        json!({"action":"import","observations":export["observations"]}),
    );
    let view = command(
        &mut target,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert!(view["record"]["summary"]["conflicts"]
        .as_array()
        .unwrap()
        .contains(&json!("invalid_correction_scope")));
}

#[test]
fn declaration_actor_controls_host_attribution_after_lower_id_reports_arrive() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let declaration=serde_json::from_value(json!({"action":"record","observations":[event("z-declaration","activity","task",json!({"event":"activity_declared","name":"Test","kind":"test","context":{}}))]})).unwrap();
    store
        .work_command(
            "project",
            &WorkActor {
                session_id: "initiating-host".into(),
                ..actor()
            },
            &declaration,
        )
        .unwrap();
    let report=serde_json::from_value(json!({"action":"record","observations":[event("a-finish","activity","task",json!({"event":"finished","outcome":"succeeded"}))]})).unwrap();
    store
        .work_command(
            "project",
            &WorkActor {
                session_id: "reporting-host".into(),
                ..actor()
            },
            &report,
        )
        .unwrap();
    let detail = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert_eq!(
        detail["record"]["summary"]["context"]["host_session_id"],
        "initiating-host"
    );
    let filtered = command(
        &mut store,
        json!({"action":"activities","query":{"host_session_id":"initiating-host"}}),
    );
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
}

#[test]
fn time_filters_follow_known_source_start_after_finish_first_ingestion() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    let mut finish = event(
        "finish",
        "activity",
        "task",
        json!({"event":"finished","outcome":"succeeded"}),
    );
    finish["occurred_at"] = json!("2026-10-24T12:10:00Z");
    command(
        &mut store,
        json!({"action":"record","observations":[finish]}),
    );
    command(
        &mut store,
        json!({"action":"record","observations":[event("declaration","activity","task",json!({"event":"activity_declared","name":"Test","kind":"test","context":{}})),event("start","activity","task",json!({"event":"started"}))]}),
    );
    let filtered = command(
        &mut store,
        json!({"action":"activities","query":{"occurred_after":"2026-10-24T11:59:00Z","occurred_before":"2026-10-24T12:01:00Z"}}),
    );
    assert_eq!(
        filtered["items"].as_array().unwrap().len(),
        1,
        "time filters must use known source start"
    );
}

#[test]
fn imported_declaration_host_attribution_preserves_original_actor() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("work.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":[event("declared","activity","task",json!({"event":"activity_declared","name":"Task","kind":"agent.task","context":{}}))]}),
    );
    let export = command(&mut store, json!({"action":"export"}));
    let mut target = Store::open(&temporary.path().join("target.db")).unwrap();
    let import =
        serde_json::from_value(json!({"action":"import","observations":export["observations"]}))
            .unwrap();
    target
        .work_command(
            "project",
            &WorkActor {
                session_id: "importing-host".into(),
                ..actor()
            },
            &import,
        )
        .unwrap();
    let detail = command(
        &mut target,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert_eq!(
        detail["record"]["summary"]["context"]["host_session_id"],
        "host-session"
    );
    assert_eq!(
        detail["record"]["observations"][0]["provenance"]["session_id"],
        "importing-host"
    );
}
