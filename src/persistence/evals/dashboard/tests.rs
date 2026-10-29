// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary disposable-postgres: typed fixture history stays inside a rolled-back transaction.
use super::*;
use crate::persistence::evals::rows::{LinkRow, RecordRow};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn input(id: &str, body: Value) -> EvalRecordInput {
    let input: EvalRecordInput = serde_json::from_value(
        json!({"id":id,"title":id,"occurred_at":"2026-10-29T12:00:00Z","body":body}),
    )
    .unwrap();
    input.validate().unwrap();
    input
}
fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "fixture".into(),
        session_id: "fixture".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}
fn seed(store: &mut Store, project: &str) {
    let mut inputs: Vec<_> = (0..128)
        .map(|i| {
            input(
                &format!("base-{i}"),
                json!({"kind":"change","motivation":{"state":"missing","reason":"Scale fixture"}}),
            )
        })
        .collect();
    inputs.extend([
        input("case",json!({"kind":"case","identity":"case","change_id":"base-0","expected_behavior":"Retain timing"})),
        input("suite",json!({"kind":"suite","identity":"suite","change_id":"base-0","cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"})),
        input("target",json!({"kind":"target","identity":"target","change_id":"base-0","components":[{"id":"worker","role":"solve","model":{"assignment":"unspecified","reason":"Fixture"}}]})),
        input("run",json!({"kind":"run","suite_id":"suite","target_id":"target","runner":"fixture","runner_version":"1","state":"planned","selection_rule":"all"})),
        input("run-complete",json!({"kind":"lifecycle","subject":{"scope":"run","id":"run"},"state":"completed"})),
    ]);
    for batch in inputs.chunks(64) {
        store.record_eval_bundle(project, batch, &actor()).unwrap();
    }
    let evidence: Vec<_> = (0..128)
        .map(|i| json!({"kind":"record","id":format!("base-{i}")}))
        .collect();
    for batch in 0..32 {
        let inputs: Vec<_> = (0..64).map(|i| input(&format!("noise-{}",batch*64+i),json!({"kind":"change","motivation":{"state":"missing","reason":"Scale fixture"},"evidence":evidence}))).collect();
        insert_history(&mut store.connection, project, &inputs);
    }
}
fn insert_history(connection: &mut DbConnection, project: &str, inputs: &[EvalRecordInput]) {
    let rows: Vec<_> = inputs
        .iter()
        .map(|input| {
            let payload = serde_json::to_string(input).unwrap();
            RecordRow {
                project_id: project.into(),
                id: input.id.clone(),
                kind: input.body.kind().as_str().into(),
                title: input.title.clone(),
                occurred_at: input.occurred_at.to_rfc3339(),
                recorded_at: input.occurred_at.to_rfc3339(),
                content_hash: blake3::hash(payload.as_bytes()).to_hex().to_string(),
                payload_json: payload,
                provenance_json: serde_json::to_string(&actor()).unwrap(),
                search_text: input.title.clone(),
            }
        })
        .collect();
    diesel::insert_into(eval_records::table)
        .values(&rows)
        .execute(connection)
        .unwrap();
    let links: Vec<_> = inputs
        .iter()
        .flat_map(|input| {
            input
                .body
                .references()
                .into_iter()
                .map(|reference| LinkRow {
                    project_id: project.into(),
                    source_id: input.id.clone(),
                    target_id: reference.link.target_id,
                    relation: reference.link.relation,
                })
        })
        .collect();
    for batch in links.chunks(512) {
        diesel::insert_into(crate::persistence::schema::eval_links::table)
            .values(batch)
            .execute(connection)
            .unwrap();
    }
}

#[test]
fn eval_lifecycle_frontier_reads_do_not_scan_unrelated_link_history() {
    let mut store = Store::connect(&std::env::var("NEXUS_TEST_DATABASE_URL").unwrap()).unwrap();
    store.connection.begin_test_transaction().unwrap();
    let background = uuid::Uuid::new_v4().to_string();
    seed(&mut store, &background);
    // Model a populated service before this project's rows enter its planner statistics.
    diesel::sql_query("ANALYZE eval_records, eval_links")
        .execute(&mut store.connection)
        .unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    seed(&mut store, &project);
    let started = Instant::now();
    for _ in 0..128 {
        let record = store.get_eval_record(&project, "run").unwrap().unwrap();
        let lifecycle = record.lifecycle.unwrap();
        assert_eq!(lifecycle.frontiers.len(), 1);
        assert_eq!(lifecycle.frontiers[0].state, ReportedState::Completed);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "128 frontier reads with 262,144 unrelated links took {elapsed:?}"
    );
    println!("128 frontier reads: {elapsed:?}");
}

#[test]
fn receipt_ordered_source_pages_stay_bounded_at_large_history() {
    let mut store = Store::connect(&std::env::var("NEXUS_TEST_DATABASE_URL").unwrap()).unwrap();
    store.connection.begin_test_transaction().unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    seed(&mut store, &project);
    for batch in 0..512 {
        let inputs: Vec<_> = (0..64).map(|offset| {
            input(&format!("run-{:05}", batch * 64 + offset), json!({"kind":"run","suite_id":"suite","target_id":"target","runner":"fixture","runner_version":"1","state":"planned","selection_rule":"all"}))
        }).collect();
        insert_history(&mut store.connection, &project, &inputs);
    }
    let started = Instant::now();
    let (records, truncated) = source_records(&mut store.connection, &project).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(records.len(), SOURCE_LIMIT);
    assert_eq!(records.first().unwrap().id, "run");
    assert_eq!(records.last().unwrap().id, "run-04094");
    assert!(truncated);
    assert!(
        elapsed < Duration::from_secs(1),
        "Receipt-ordered source read took {elapsed:?}"
    );
    println!("32,769 source records: {elapsed:?}");
}

#[test]
fn eval_dashboard_keeps_a_bounded_dimension_query_budget() {
    let mut store = Store::connect(&std::env::var("NEXUS_TEST_DATABASE_URL").unwrap()).unwrap();
    store.connection.begin_test_transaction().unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    seed(&mut store, &project);
    let mut inputs = vec![];
    for i in 0..64 {
        inputs.push(input(&format!("suite-{i}"), json!({"kind":"suite","identity":"suite","change_id":"base-0","cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"})));
        inputs.push(input(&format!("target-{i}"), json!({"kind":"target","identity":"target","change_id":"base-0","components":[{"id":"worker","role":"solve","model":{"assignment":"unspecified","reason":"Fixture"}}]})));
        inputs.push(input(&format!("run-{i}"), json!({"kind":"run","suite_id":format!("suite-{i}"),"target_id":format!("target-{i}"),"runner":"fixture","runner_version":"1","state":"planned","selection_rule":"all"})));
    }
    for batch in inputs.chunks(64) {
        insert_history(&mut store.connection, &project, batch);
    }
    let queries = crate::persistence::test_support::capture_queries(&mut store.connection);
    let dashboard = store.eval_dashboard(&project, None).unwrap();
    assert_eq!(dashboard.run_count, 65);
    assert_eq!(dashboard.groups.len(), 1);
    assert_eq!(dashboard.groups[0].statuses.get("planned"), Some(&64));
    assert_eq!(dashboard.groups[0].statuses.get("completed"), Some(&1));
    let count = queries.lock().unwrap().len();
    assert!(count <= 150, "65 grouped runs used {count} queries");
    println!("65 grouped runs: {count} queries");
}
