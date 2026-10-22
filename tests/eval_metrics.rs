// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary typed-metric-fixtures: arbitrary measured values remain exact caller-supplied text.
use nexus::evals::{EvalProvenance, EvalQuery, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::{json, Value};
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::{record, revision as definition};

fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "metrics".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}

fn metric(id: &str) -> EvalRecordInput {
    definition(
        id,
        "metric",
        json!({
            "name":"caller objective","description":"A caller-defined objective, including cost, speed, accuracy or categories",
            "unit":"widgets / arbitrary unit","value_format":"Exact caller text; this fixture deliberately accepts any representation",
            "value_schema":"calculation-source","calculation":{
                "description":"Use the pinned caller calculation","formula":"caller supplied formula",
                "source_artifacts":["calculation-source"],"population":"Explicit measurement inputs",
                "aggregation":"Caller-defined","exclusions":"No hidden exclusions","missing_data":"Report unavailable with a reason"
            },"interpretation":"A 10 to 7 change means three fewer widgets; Nexus makes no optimization decision"
        }),
    )
}

fn definitions() -> Vec<EvalRecordInput> {
    vec![
        record(
            "change",
            json!({"kind":"change","motivation":{"state":"missing","reason":"Historical caller fixture"}}),
        ),
        record(
            "calculation-source",
            json!({"kind":"artifact","media_type":"text/plain","byte_size":100,"backings":[{"kind":"git_file","repository":"caller-repo","commit":"a".repeat(40),"object_format":"sha1","path":"metrics/definition.txt","entry_mode":"100644"}]}),
        ),
        definition(
            "repo",
            "repository",
            json!({"repository":"caller-repo","commit":"a".repeat(40),"object_format":"sha1"}),
        ),
        definition(
            "planned",
            "model",
            json!({"label":"planned","provider":"test","requested_model":"planned"}),
        ),
        definition(
            "actual",
            "model",
            json!({"label":"actual","provider":"test","requested_model":"actual"}),
        ),
        definition(
            "case",
            "case",
            json!({"expected_behavior":"Retain the caller-defined result"}),
        ),
        definition(
            "suite",
            "suite",
            json!({"cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller-defined"}),
        ),
        definition(
            "target",
            "target",
            json!({"components":[{"id":"worker","role":"solve","repository_id":"repo","model":{"assignment":"fixed","model_id":"planned"}}]}),
        ),
        metric("metric"),
        metric("other-metric"),
    ]
}

fn executions() -> Vec<EvalRecordInput> {
    vec![
        record(
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"}),
        ),
        record(
            "trial",
            json!({"kind":"trial","run_id":"run","case_id":"case","replicate":0,"state":"completed"}),
        ),
        record(
            "attempt",
            json!({"kind":"attempt","trial_id":"trial","state":"completed","started_at":"2026-10-22T12:00:00Z"}),
        ),
        record(
            "call",
            json!({"kind":"call","attempt_id":"attempt","component":"worker","instance":"worker-1","sequence":0,"model_id":"actual"}),
        ),
        record(
            "trace",
            json!({"kind":"trace","attempt_id":"attempt","call_id":"call","sequence":0,"detail":{"event":"outcome","state":"completed","evidence_artifacts":[]}}),
        ),
    ]
}

fn measurement(id: &str, subject: Value, value: Value) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"measurement","metric_id":"metric","subject":subject,"value":value,"explanation":"Caller-reported result","inputs":["call"],"evidence":["calculation-source"]}),
    )
}

fn fixture() -> (tempfile::TempDir, Store) {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    let mut records = definitions();
    records.extend(executions());
    records.reverse();
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    (temporary, store)
}

fn body(store: &mut Store, id: &str) -> Value {
    serde_json::to_value(store.get_eval_record("p", id).unwrap().unwrap().input.body).unwrap()
}

fn exact_measurements() -> Vec<EvalRecordInput> {
    [
        ("run-value",json!({"scope":"run","id":"run"}),"7.00"),
        ("trial-value",json!({"scope":"trial","id":"trial"}),"900719925474099312345678901234567890"),
        ("attempt-value",json!({"scope":"attempt","id":"attempt"}),"p50,p95\n0.01,0.03\n"),
        ("call-value",json!({"scope":"call","id":"call"}),"{not valid JSON, despite the caller's declared format}"),
        ("component-value",json!({"scope":"component","attempt_id":"attempt","instance":"worker-1","call_ids":["call"]}),"true"),
        ("routing-value",json!({"scope":"routing","trace_id":"trace"}),"preferred category"),
    ].into_iter().map(|(id,subject,text)| measurement(id,subject,json!({"storage":"inline","text":text}))).collect()
}

#[test]
fn arbitrary_metric_values_and_calculation_meaning_survive_snapshot_without_coercion() {
    let (temporary, mut store) = fixture();
    let records = exact_measurements();
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let manifest = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut restored = Store::open(&temporary.path().join("restored.db")).unwrap();
    restored.import_streamed_snapshot(snapshot.path()).unwrap();
    assert_eq!(
        restored.write_streamed_snapshot(Vec::new()).unwrap(),
        manifest
    );
    for record in records {
        assert_eq!(
            body(&mut restored, &record.id),
            serde_json::to_value(record.body).unwrap()
        );
    }
    let history = restored.why_eval_record("p", "run", 64).unwrap();
    for id in ["run-value", "metric", "calculation-source", "change"] {
        assert!(
            history.records.iter().any(|record| record.input.id == id),
            "missing metric context {id}"
        );
    }
    assert!(body(&mut restored, "metric")["interpretation"]
        .as_str()
        .unwrap()
        .contains("10 to 7"));
}

fn rejected(store: &mut Store, records: &[EvalRecordInput]) {
    assert!(store.record_eval_bundle("p", records, &actor()).is_err());
    for record in records {
        assert!(store.get_eval_record("p", &record.id).unwrap().is_none());
    }
}

#[test]
fn measurement_corrections_keep_the_metric_and_subject_and_cannot_cycle() {
    let (_temporary, mut store) = fixture();
    let original = measurement(
        "original",
        json!({"scope":"run","id":"run"}),
        json!({"storage":"inline","text":"10"}),
    );
    store
        .record_eval_bundle("p", &[original], &actor())
        .unwrap();
    for fields in [
        json!({"metric_id":"other-metric"}),
        json!({"subject":{"scope":"trial","id":"trial"}}),
    ] {
        let mut invalid = body(&mut store, "original");
        invalid
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        invalid["replaces"] = json!("original");
        rejected(&mut store, &[record("invalid", invalid)]);
    }
    let mut corrected = body(&mut store, "original");
    corrected["replaces"] = json!("original");
    corrected["value"] = json!({"storage":"inline","text":"7"});
    store
        .record_eval_bundle("p", &[record("corrected", corrected)], &actor())
        .unwrap();
    let mut cycle = body(&mut store, "original");
    cycle["replaces"] = json!("cycle-b");
    let mut other = cycle.clone();
    other["replaces"] = json!("cycle-a");
    rejected(
        &mut store,
        &[record("cycle-a", cycle), record("cycle-b", other)],
    );
    let history = store.why_eval_record("p", "original", 64).unwrap();
    assert!(history
        .records
        .iter()
        .any(|record| record.input.id == "corrected"));
    assert_eq!(body(&mut store, "original")["value"]["text"], "10");
}

#[test]
fn measurement_component_filters_use_only_pinned_calls_and_reject_wrong_instances() {
    let (_temporary, mut store) = fixture();
    store
        .record_eval_bundle("p", &exact_measurements(), &actor())
        .unwrap();
    for (model, expected) in [
        (
            "actual",
            vec!["call-value", "component-value", "routing-value"],
        ),
        ("planned", vec!["attempt-value", "run-value", "trial-value"]),
    ] {
        let query: EvalQuery = serde_json::from_value(
            json!({"kind":"measurement","model":model,"repository":"caller-repo"}),
        )
        .unwrap();
        let ids: Vec<_> = store
            .list_eval_records("p", &query)
            .unwrap()
            .records
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, expected);
    }
    let invalid = measurement(
        "wrong-instance",
        json!({"scope":"component","attempt_id":"attempt","instance":"sibling","call_ids":["call"]}),
        json!({"storage":"inline","text":"7"}),
    );
    rejected(&mut store, &[invalid]);
}

fn comparison(id: &str, fields: Value) -> EvalRecordInput {
    let mut value = json!({"kind":"comparison","baseline_ids":["run"],"candidate_ids":["run"],"grade_ids":[],"population":"Pinned reported population","exclusions":"None","validity":"matched","differences":[],"result":"Caller comparison","uncertainty":"Not estimated"});
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    record(id, value)
}

#[test]
fn arbitrary_comparisons_link_metrics_and_measurements_without_a_numeric_metric() {
    let (_temporary, mut store) = fixture();
    let records = [
        measurement(
            "artifact-value",
            json!({"scope":"run","id":"run"}),
            json!({"storage":"artifact","artifact_id":"calculation-source"}),
        ),
        measurement(
            "unavailable-value",
            json!({"scope":"run","id":"run"}),
            json!({"storage":"unavailable","reason":"Caller did not capture this value"}),
        ),
    ];
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let value = comparison(
        "comparison",
        json!({"metric_ids":["metric"],"measurement_ids":["artifact-value","unavailable-value"]}),
    );
    store.record_eval_bundle("p", &[value], &actor()).unwrap();
    assert!(body(&mut store, "comparison").get("metric").is_none());
    let history = store.why_eval_record("p", "comparison", 64).unwrap();
    assert!(history
        .records
        .iter()
        .any(|record| record.input.id == "metric"));
    rejected(&mut store, &[comparison("missing-metric", json!({}))]);
    rejected(
        &mut store,
        &[comparison("wrong-kind", json!({"metric_ids":["run"]}))],
    );
}

#[test]
fn legacy_numeric_comparison_keeps_its_original_serialized_body_shape() {
    let metric = json!({"name":"pass","unit":"ratio","direction":"higher","minimum":null,"maximum":null,"threshold":null});
    let legacy = comparison("legacy", json!({"metric":metric}));
    let serialized = serde_json::to_value(legacy.body).unwrap();
    assert_eq!(serialized["metric"], metric);
    assert!(serialized.get("metric_ids").is_none());
    assert!(serialized.get("measurement_ids").is_none());
}
