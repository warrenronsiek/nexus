// @feature evals
// @spec docs/features/evals.md
// @boundary typed-eval-fixtures
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::record;
use nexus::evals::{EvalProvenance, EvalQuery, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::json;

fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "session".into(),
        model_id: None,
        config_hash: "config".into(),
    }
}

fn change(id: &str) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"change","motivation":{"state":"recorded","reason":"observed_failure","problem":"Agent ignored the available tool","expected_behavior":"Use the supplied tool","rationale":"Retain a regression case after the incident","provenance":"contemporaneous"},"evidence":[]}),
    )
}

#[test]
fn eval_history_preserves_motivation_and_rejects_conflicts_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let case = record(
        "case-1",
        json!({"kind":"case","identity":"adoption","parents":[],"change_id":"change-1","inputs":[],"expected_behavior":"Use the supplied tool","assertions":[],"tags":["regression"]}),
    );
    let records = vec![case, change("change-1")];
    assert_eq!(
        store
            .record_eval_bundle("p", &records, &actor())
            .unwrap()
            .inserted,
        2
    );
    assert_eq!(
        store
            .record_eval_bundle("p", &records, &actor())
            .unwrap()
            .replayed,
        2
    );
    let why = store.why_eval_record("p", "case-1", 32).unwrap();
    assert_eq!(why.records.len(), 2);
    assert!(serde_json::to_string(&why)
        .unwrap()
        .contains("Retain a regression"));
    let mut conflicting = records[1].clone();
    conflicting.title = "altered".into();
    assert!(store
        .record_eval_bundle("p", &[change("new-change"), conflicting], &actor())
        .is_err());
    assert!(store.get_eval_record("p", "new-change").unwrap().is_none());
    assert!(store
        .get_eval_record("other-project", "case-1")
        .unwrap()
        .is_none());
    let page = store
        .list_eval_records(
            "p",
            &EvalQuery {
                search: Some("adoption".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.records.len(), 1);
}

#[test]
fn eval_compositions_filter_every_model_and_preserve_separate_grading_passes() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let mut records = composition_sources();
    records.extend(composition_measurement());
    records.push(composition_target());
    records.extend(composition_execution());
    records.reverse();
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let query: EvalQuery =
        serde_json::from_value(json!({"kind":"run","repository":"dispatch","model":"strong"}))
            .unwrap();
    assert_eq!(
        store.list_eval_records("p", &query).unwrap().records[0].id,
        "run"
    );
    let grades: EvalQuery = serde_json::from_value(json!({"kind":"grade"})).unwrap();
    assert_eq!(
        store.list_eval_records("p", &grades).unwrap().records.len(),
        2
    );
    let why = store.why_eval_record("p", "grade-2", 64).unwrap();
    assert!(why.records.iter().any(|r| r.input.id == "grade-1"));
    assert!(why.records.iter().any(|r| r.input.id == "change"));
    assert!(!why.truncated);
}

#[test]
fn eval_bundle_rejects_dangling_links_and_pins_git_without_uploads() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let dangling = record(
        "case",
        json!({"kind":"case","identity":"x","parents":[],"change_id":"missing","inputs":[],"expected_behavior":"behavior","assertions":[],"tags":[]}),
    );
    assert!(store
        .record_eval_bundle("p", &[dangling], &actor())
        .is_err());
    let artifact = record(
        "fixture",
        json!({"kind":"artifact","media_type":"image/png","byte_size":123,"digest":null,"backings":[{"kind":"git_file","repository":"repo","commit":"a".repeat(40),"object_format":"sha1","path":"fixtures/image.png","blob_oid":null,"entry_mode":"100644"}]}),
    );
    store
        .record_eval_bundle("p", &[artifact], &actor())
        .unwrap();
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let original = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    target.import_streamed_snapshot(snapshot.path()).unwrap();
    assert!(target.get_eval_record("p", "fixture").unwrap().is_some());
    assert_eq!(
        target.write_streamed_snapshot(Vec::new()).unwrap(),
        original
    );
}

#[test]
fn eval_blob_is_readable_only_after_verified_idempotent_upload() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let bytes = b"generated response";
    let digest = format!("{:x}", Sha256::digest(bytes));
    store
        .begin_eval_blob("p", &digest, bytes.len() as u64)
        .unwrap();
    assert!(store.seal_eval_blob("p", &digest).is_err());
    store.put_eval_blob_chunk("p", &digest, 0, bytes).unwrap();
    store.put_eval_blob_chunk("p", &digest, 0, bytes).unwrap();
    assert!(store
        .put_eval_blob_chunk("p", &digest, 0, b"different")
        .is_err());
    assert!(store
        .get_eval_blob_chunk("p", &digest, 0)
        .unwrap()
        .is_none());
    assert!(store.seal_eval_blob("p", &digest).unwrap().verified);
    assert_eq!(
        store.get_eval_blob_chunk("p", &digest, 0).unwrap().unwrap(),
        bytes
    );
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let manifest = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    target.import_streamed_snapshot(snapshot.path()).unwrap();
    assert_eq!(
        target
            .get_eval_blob_chunk("p", &digest, 0)
            .unwrap()
            .unwrap(),
        bytes
    );
    assert_eq!(
        target.write_streamed_snapshot(Vec::new()).unwrap(),
        manifest
    );
}

#[test]
fn eval_pagination_freezes_the_original_population_during_later_ingestion() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    store
        .record_eval_bundle("p", &[change("b"), change("d"), change("f")], &actor())
        .unwrap();
    let first = store
        .list_eval_records(
            "p",
            &EvalQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(first.records[0].id, "b");
    store
        .record_eval_bundle("p", &[change("a"), change("z")], &actor())
        .unwrap();
    let remaining = store
        .list_eval_records(
            "p",
            &EvalQuery {
                after: first.next_cursor,
                limit: Some(64),
                ..Default::default()
            },
        )
        .unwrap();
    let ids: Vec<_> = remaining.records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["d", "f"]);
}

#[test]
fn eval_model_filters_follow_execution_bindings_instead_of_motivation_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let mut records = model_history_sources();
    records.extend(model_generation("old"));
    records.extend(model_generation("new"));
    store.record_eval_bundle("p", &records, &actor()).unwrap();
    let query: EvalQuery = serde_json::from_value(json!({"kind":"run","model":"old"})).unwrap();
    let page = store.list_eval_records("p", &query).unwrap();
    assert_eq!(
        page.records
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-old"]
    );
}

#[test]
fn eval_why_retains_original_motivation_and_finds_later_corrections() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let case = record(
        "case",
        json!({"kind":"case","identity":"case","parents":[],"change_id":"original","expected_behavior":"Use the supplied tool"}),
    );
    store
        .record_eval_bundle("p", &[change("original"), case], &actor())
        .unwrap();
    let original = store.get_eval_record("p", "original").unwrap().unwrap();
    let mut correction = change("later");
    correction.occurred_at = "2026-10-23T12:00:00Z".parse().unwrap();
    if let nexus::evals::EvalBody::Change(body) = &mut correction.body {
        body.supersedes.push("original".into());
    }
    store
        .record_eval_bundle("p", &[correction], &actor())
        .unwrap();
    let why = store.why_eval_record("p", "case", 64).unwrap();
    assert!(why.records.iter().any(|r| r.input.id == "later"));
    assert_eq!(
        serde_json::to_string(&store.get_eval_record("p", "original").unwrap().unwrap()).unwrap(),
        serde_json::to_string(&original).unwrap()
    );
    assert!(
        why.records
            .iter()
            .find(|r| r.input.id == "later")
            .unwrap()
            .input
            .occurred_at
            > original.input.occurred_at
    );
}

fn revision(identity: &str) -> serde_json::Value {
    json!({"identity":identity,"parents":[],"change_id":"change"})
}

fn composition_sources() -> Vec<EvalRecordInput> {
    let mut records = vec![change("change")];
    let mut repository = revision("repo-history");
    repository["kind"] = json!("repository");
    repository["repository"] = json!("dispatch");
    repository["commit"] = json!("a".repeat(40));
    repository["object_format"] = json!("sha1");
    records.push(record("repo", repository));
    for name in ["fast", "strong"] {
        let mut model = revision(name);
        model["kind"] = json!("model");
        model["label"] = json!(name);
        model["provider"] = json!("test");
        model["requested_model"] = json!(name);
        records.push(record(name, model));
    }
    let mut case = revision("case-history");
    case["kind"] = json!("case");
    case["expected_behavior"] = json!("Route to the right tool");
    records.push(record("case", case));
    records
}
fn composition_measurement() -> Vec<EvalRecordInput> {
    let mut records = vec![];
    let metric = json!({"name":"accuracy","unit":"ratio","direction":"higher","minimum":0.0,"maximum":1.0,"threshold":0.8});
    let mut grader = revision("grader-history");
    grader["kind"] = json!("grader");
    grader["grading"] = json!({"method":"human","instructions":"Assess the final task outcome"});
    grader["metric"] = metric.clone();
    records.push(record("grader", grader));
    let mut suite = revision("suite-history");
    suite["kind"] = json!("suite");
    suite["cases"] =
        json!([{"case_id":"case","grader_ids":["grader"],"weight":1.0,"split":"development"}]);
    suite["aggregation"] = json!("mean");
    records.push(record("suite", suite));
    records
}
fn composition_target() -> EvalRecordInput {
    let mut target = revision("target-history");
    target["kind"] = json!("target");
    target["components"] = json!([
        {"id":"router","role":"dispatch","repository_id":"repo","model":{"assignment":"fixed","model_id":"fast"},"artifacts":[]},
        {"id":"worker","role":"review","repository_id":"repo","model":{"assignment":"fixed","model_id":"strong"},"artifacts":[]}
    ]);
    record("target", target)
}
fn composition_execution() -> Vec<EvalRecordInput> {
    let mut records = vec![];
    records.push(record("run",json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller-owned","runner_version":"1","state":"completed","selection_rule":"all samples","selected_cases":["case"]})));
    records.push(record(
        "trial",
        json!({"kind":"trial","run_id":"run","case_id":"case","replicate":0,"state":"completed"}),
    ));
    records.push(record("attempt",json!({"kind":"attempt","trial_id":"trial","state":"completed","started_at":"2026-10-22T12:00:00Z","finished_at":"2026-10-22T12:01:00Z"})));
    for (id, score, replaces) in [("grade-1", 0.0, None), ("grade-2", 1.0, Some("grade-1"))] {
        records.push(record(id,json!({"kind":"grade","subject":{"scope":"attempt","id":"attempt"},"grader_id":"grader","pass_id":id,"state":"completed","score":score,"explanation":"Caller assessment","replaces":replaces})));
    }
    records
}

fn model_history_sources() -> Vec<EvalRecordInput> {
    let revision = |kind: &str, id: &str| json!({"kind":kind,"identity":id,"parents":[],"change_id":"change-old"});
    let mut records = vec![change("change-old")];
    for model in ["old", "new"] {
        let mut body = revision("model", model);
        body["label"] = json!(model);
        body["provider"] = json!("test");
        body["requested_model"] = json!(model);
        records.push(record(model, body));
    }
    let mut suite = revision("suite", "suite");
    suite["cases"] = json!([]);
    suite["aggregation"] = json!("mean");
    records.push(record("suite", suite));
    records
}
fn model_generation(model: &str) -> Vec<EvalRecordInput> {
    let mut records = vec![];
    let change_id = format!("change-{model}");
    if model == "new" {
        let mut motivating = change(&change_id);
        motivating.body=serde_json::from_value(json!({"kind":"change","motivation":{"state":"recorded","reason":"observed_failure","problem":"Old target failed","expected_behavior":"New model succeeds","rationale":"Try the alternative model","provenance":"contemporaneous"},"evidence":[{"kind":"record","id":"run-old"}]})).unwrap();
        records.push(motivating);
    }
    let target = json!({"kind":"target","identity":"target","parents":if model=="new" {vec!["target-old"]}else{vec![]},"change_id":change_id,"components":[{"id":"agent","role":"solve","model":{"assignment":"fixed","model_id":model},"artifacts":[]}]});
    records.push(record(&format!("target-{model}"), target));
    records.push(record(&format!("run-{model}"),json!({"kind":"run","target_id":format!("target-{model}"),"suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"})));
    records
}
