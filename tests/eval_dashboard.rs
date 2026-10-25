// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary fixture-json: caller result values remain exact text beside optional chart approximations.
#![cfg(feature = "sqlite")]
use nexus::evals::{EvalProvenance, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::{json, Value};
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::{record, revision};
fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "host".into(),
        model_id: None,
        config_hash: "test".into(),
    }
}
fn definitions() -> Vec<EvalRecordInput> {
    vec![
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
            "model",
            "model",
            json!({"label":"test-model","provider":"fixture","requested_model":"test-model"}),
        ),
        revision("case", "case", json!({"expected_behavior":"Answer"})),
        revision(
            "suite",
            "suite",
            json!({"cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"}),
        ),
        revision(
            "target",
            "target",
            json!({"components":[{"id":"worker","role":"answer","repository_id":"repo","model":{"assignment":"fixed","model_id":"model"}}]}),
        ),
        revision(
            "metric",
            "metric",
            json!({"name":"Latency","description":"Caller latency","unit":"seconds","value_format":"decimal","calculation":{"description":"Caller elapsed clock","population":"run","aggregation":"caller","exclusions":"none","missing_data":"unavailable"}}),
        ),
        record(
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"}),
        ),
    ]
}
fn measured(id: &str, text: &str, replaces: Option<&str>) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"measurement","metric_id":"metric","subject":{"scope":"run","id":"run"},"value":{"storage":"inline","text":text},"explanation":"Caller result","replaces":replaces}),
    )
}
#[test]
fn eval_dashboard_preserves_metric_meaning_frontiers_and_opaque_values() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("eval.db")).unwrap();
    store
        .record_eval_bundle("p", &definitions(), &actor())
        .unwrap();
    store
        .record_eval_bundle(
            "p",
            &[
                measured("old", "10", None),
                measured("current", "7", Some("old")),
                measured("opaque", "900719925474099312345", None),
            ],
            &actor(),
        )
        .unwrap();
    let overview = store.eval_dashboard("p", None).unwrap();
    assert_eq!(overview.groups.len(), 1);
    let group = &overview.groups[0];
    assert_eq!(group.suite_identity, "suite");
    assert_eq!(group.models, vec!["test-model"]);
    let selected =
        serde_json::to_value(store.eval_dashboard("p", Some(&group.id)).unwrap()).unwrap();
    assert_eq!(selected["series"][0]["unit"], "seconds");
    assert_eq!(selected["series"][0]["superseded_count"], 1);
    let points = selected["series"][0]["points"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    let precise = points
        .iter()
        .find(|point| point["record_id"] == "opaque")
        .unwrap();
    assert_eq!(precise["numeric_value"], Value::Null);
    assert_eq!(precise["value"]["text"], "900719925474099312345");
    assert_eq!(selected["truncated"], false);
}
#[test]
fn numeric_looking_categories_do_not_become_chart_quantities() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("category.db")).unwrap();
    let mut records = definitions();
    if let nexus::evals::EvalBody::Metric(metric) = &mut records[6].body {
        metric.value_format = "categorical identifier".into();
    }
    records.push(measured("category", "7", None));
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let groups = store.eval_dashboard("p", None).unwrap();
    let selected = store
        .eval_dashboard("p", Some(&groups.groups[0].id))
        .unwrap();
    assert_eq!(selected.series[0].points[0].numeric_value, None);
    assert_eq!(
        serde_json::to_value(&selected.series[0].points[0].value).unwrap()["text"],
        "7"
    );
}
