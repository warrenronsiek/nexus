// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary typed-component-history-fixtures
use nexus::evals::{EvalProvenance, EvalQuery, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::{json, Value};

fn record(id: &str, body: Value) -> EvalRecordInput {
    serde_json::from_value(
        json!({"id":id,"title":id,"occurred_at":"2026-10-22T12:00:00Z","body":body}),
    )
    .unwrap()
}
fn revision(id: &str, kind: &str, fields: Value) -> EvalRecordInput {
    let mut body = json!({"kind":kind,"identity":id,"change_id":"change"});
    body.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    record(id, body)
}
fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "component-dimensions".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}
fn definitions() -> Vec<EvalRecordInput> {
    let mut records = vec![record(
        "change",
        json!({"kind":"change","motivation":{"state":"missing","reason":"Historical fixture"}}),
    )];
    records.push(revision(
        "repo",
        "repository",
        json!({"repository":"dispatch","commit":"a".repeat(40),"object_format":"sha1"}),
    ));
    for id in ["router-model", "worker-model", "fallback-model"] {
        records.push(revision(
            id,
            "model",
            json!({"label":id,"provider":"test","requested_model":id}),
        ));
    }
    records.push(revision(
        "case",
        "case",
        json!({"expected_behavior":"Route the task"}),
    ));
    records.push(revision("grader","grader",json!({"grading":{"method":"human","instructions":"Inspect the component outcome"},"metric":{"name":"pass","unit":"ratio","direction":"higher"}})));
    records.push(revision("suite","suite",json!({"cases":[{"case_id":"case","grader_ids":["grader"],"weight":1,"split":"development"}],"aggregation":"mean"})));
    records
}
fn execution() -> Vec<EvalRecordInput> {
    vec![
        revision(
            "target",
            "target",
            json!({"components":[
                {"id":"router","role":"dispatch","repository_id":"repo","model":{"assignment":"fixed","model_id":"router-model"}},
                {"id":"worker","role":"solve","repository_id":"repo","model":{"assignment":"fixed","model_id":"worker-model"}}
            ]}),
        ),
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
    ]
}
fn call(id: &str, instance: &str, model: &str) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"call","attempt_id":"attempt","component":instance,"instance":instance,"sequence":0,"model_id":model}),
    )
}
fn grade(id: &str, instance: &str, calls: &[&str]) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"grade","subject":{"scope":"component","attempt_id":"attempt","instance":instance,"call_ids":calls},"grader_id":"grader","pass_id":id,"state":"completed","score":1,"explanation":"Component-local result"}),
    )
}
fn matching(store: &mut Store, query: Value) -> Vec<String> {
    let query: EvalQuery = serde_json::from_value(query).unwrap();
    store
        .list_eval_records("p", &query)
        .unwrap()
        .records
        .into_iter()
        .map(|r| r.id)
        .collect()
}

#[test]
fn component_grade_filters_follow_actual_instance_models_in_the_same_bundle() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    let mut records = definitions();
    records.extend(execution());
    records.extend([
        call("router-call", "router-instance", "router-model"),
        call("worker-call", "worker-instance", "fallback-model"),
        grade("worker-grade", "worker-instance", &["worker-call"]),
    ]);
    records.reverse();
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    assert_eq!(
        matching(&mut store, json!({"kind":"grade","model":"fallback-model"})),
        vec!["worker-grade"]
    );
    assert!(matching(&mut store, json!({"kind":"grade","model":"router-model"})).is_empty());
    assert!(matching(&mut store, json!({"kind":"grade","model":"worker-model"})).is_empty());
}

#[test]
fn component_call_evidence_is_explicit_and_empty_pins_remain_unknown() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    let mut records = definitions();
    records.extend(execution());
    records.push(call("router-call", "router-instance", "router-model"));
    records.push(call("z-worker-call", "worker-instance", "worker-model"));
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    store
        .record_eval_bundle(
            "p",
            &[
                grade("known-grade", "worker-instance", &["z-worker-call"]),
                grade("unknown-grade", "worker-instance", &[]),
            ],
            &actor(),
        )
        .unwrap();
    assert_eq!(
        matching(&mut store, json!({"kind":"grade","model":"worker-model"})),
        vec!["known-grade"]
    );
    assert!(matching(&mut store, json!({"kind":"grade","model":"router-model"})).is_empty());
    assert_eq!(
        matching(&mut store, json!({"kind":"grade","repository":"dispatch"})),
        vec!["known-grade", "unknown-grade"]
    );
}

#[test]
fn component_grade_cannot_pin_a_sibling_instance_or_another_attempt() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    let mut records = definitions();
    records.extend(execution());
    records.push(call("router-call", "router-instance", "router-model"));
    records.push(record("other-attempt",json!({"kind":"attempt","trial_id":"trial","state":"completed","started_at":"2026-10-22T12:00:00Z"})));
    records.push(record("other-call",json!({"kind":"call","attempt_id":"other-attempt","component":"worker","instance":"worker-instance","sequence":0,"model_id":"worker-model"})));
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    for call_id in ["router-call", "other-call"] {
        let id = format!("invalid-{call_id}");
        assert!(store
            .record_eval_bundle("p", &[grade(&id, "worker-instance", &[call_id])], &actor())
            .is_err());
        assert!(store.get_eval_record("p", &id).unwrap().is_none());
    }
}
