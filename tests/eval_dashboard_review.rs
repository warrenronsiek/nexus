// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary fixture-json: eval set identity must retain component model assignments and selected summaries through display bounds.
#![cfg(feature = "sqlite")]
use nexus::evals::{EvalProvenance, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::json;
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::{record, revision};
fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "dashboard-review".into(),
        session_id: "fixture".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}
fn definitions() -> Vec<EvalRecordInput> {
    let mut result = vec![
        record(
            "change",
            json!({"kind":"change","motivation":{"state":"missing","reason":"Historical fixture"}}),
        ),
        revision(
            "repo",
            "repository",
            json!({"repository":"router","commit":"a".repeat(40),"object_format":"sha1"}),
        ),
        revision(
            "case",
            "case",
            json!({"expected_behavior":"Route and answer"}),
        ),
        revision(
            "suite",
            "suite",
            json!({"cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"}),
        ),
    ];
    for id in ["model-a", "model-b"] {
        result.push(revision(
            id,
            "model",
            json!({"label":id,"provider":"fixture","requested_model":id}),
        ));
    }
    result
}
fn target(id: &str, identity: &str, router: &str, worker: &str) -> EvalRecordInput {
    revision(
        id,
        "target",
        json!({"identity":identity,"components":[
            {"id":"router","role":"dispatch","repository_id":"repo","model":{"assignment":"fixed","model_id":router}},
            {"id":"worker","role":"answer","repository_id":"repo","model":{"assignment":"fixed","model_id":worker}}
        ]}),
    )
}
fn run(id: &str, target_id: &str, time: &str) -> EvalRecordInput {
    let mut result = record(
        id,
        json!({"kind":"run","target_id":target_id,"suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"}),
    );
    result.occurred_at = time.parse().unwrap();
    result
}
#[test]
fn eval_dashboard_distinguishes_models_assigned_to_different_agent_roles() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("eval.db")).unwrap();
    let mut records = definitions();
    records.extend([
        target("target-a", "router-and-worker", "model-a", "model-b"),
        target("target-b", "router-and-worker", "model-b", "model-a"),
        run("run-a", "target-a", "2026-10-22T12:00:00Z"),
        run("run-b", "target-b", "2026-10-23T12:00:00Z"),
    ]);
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let dashboard = store.eval_dashboard("p", None).unwrap();
    assert_eq!(dashboard.groups.len(), 2, "the same model set with swapped router/worker assignments is a different agent composition");
}
#[test]
fn eval_dashboard_retains_selected_group_summary_outside_recent_group_window() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("eval.db")).unwrap();
    let mut records = definitions();
    records.extend([
        target("target-old", "old", "model-a", "model-b"),
        run("run-old", "target-old", "2026-10-22T12:00:00Z"),
    ]);
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let selected_id = store.eval_dashboard("p", None).unwrap().groups[0]
        .id
        .clone();
    for index in 0..512 {
        let target_id = format!("target-{index}");
        store
            .record_eval_bundle(
                "p",
                &[
                    target(&target_id, &target_id, "model-a", "model-b"),
                    run(&format!("run-{index}"), &target_id, "2026-10-23T12:00:00Z"),
                ],
                &actor(),
            )
            .unwrap();
    }
    let selected = store.eval_dashboard("p", Some(&selected_id)).unwrap();
    assert!(selected.truncated);
    assert!(
        selected.groups.iter().any(|group| group.id == selected_id),
        "selected group metadata must survive the display cap"
    );
}
