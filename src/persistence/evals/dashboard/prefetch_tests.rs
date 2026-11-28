// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary synthetic-eval-records: bounded fixture definitions exercise the real projection's query count.
use super::*;
use crate::persistence::test_support::capture_queries;
use serde_json::json;

#[test]
fn repeated_run_definitions_are_loaded_in_batches() {
    #[cfg(feature = "sqlite")]
    let mut store = Store::open_memory().unwrap();
    #[cfg(feature = "postgres")]
    let mut store = {
        let mut store = Store::connect(&std::env::var("NEXUS_TEST_DATABASE_URL").unwrap()).unwrap();
        store.connection.begin_test_transaction().unwrap();
        store
    };
    let project = uuid::Uuid::new_v4().to_string();
    let actor = EvalProvenance {
        agent: "fixture".into(),
        session_id: "fixture".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    seed(&mut store, &project, &actor);
    let queries = capture_queries(&mut store.connection);
    let started = std::time::Instant::now();
    let dashboard = store.eval_dashboard(&project, None).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(dashboard.run_count, 65);
    assert_eq!(dashboard.groups.len(), 1);
    let count = queries.lock().unwrap().len();
    assert!(
        count <= 40,
        "65 runs required {count} queries in {elapsed:?}"
    );
    println!("65 runs: {count} queries, {elapsed:?}");
}

fn seed(store: &mut Store, project: &str, actor: &EvalProvenance) {
    let record = |id: String, body: serde_json::Value| {
        serde_json::from_value::<EvalRecordInput>(
            json!({"id":id, "title":id, "occurred_at":"2026-11-27T00:00:00Z", "body":body}),
        )
        .unwrap()
    };
    let mut inputs = vec![
        record(
            "change".into(),
            json!({"kind":"change","motivation":{"state":"missing","reason":"Fixture"}}),
        ),
        record(
            "case".into(),
            json!({"kind":"case","identity":"case","change_id":"change","expected_behavior":"Answer"}),
        ),
    ];
    for i in 0..65 {
        inputs.extend([
            record(format!("suite-{i}"), json!({"kind":"suite","identity":"suite","change_id":"change","cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"})),
            record(format!("target-{i}"),json!({"kind":"target","identity":"target","change_id":"change","components":[{"id":"worker","role":"solve","model":{"assignment":"unspecified","reason":"Fixture"}}]})),
            record(format!("run-{i}"),json!({"kind":"run","suite_id":format!("suite-{i}"),"target_id":format!("target-{i}"),"runner":"fixture","runner_version":"1","state":"completed","selection_rule":"all"})),
        ]);
    }
    for page in inputs.chunks(64) {
        store.record_eval_bundle(project, page, actor).unwrap();
    }
}
