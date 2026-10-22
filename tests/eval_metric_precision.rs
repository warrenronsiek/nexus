// @feature evals
// @spec docs/features/evals.md
// @boundary caller-defined-metric-fixtures
use nexus::evals::{EvalProvenance, EvalQuery, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::{json, Value};
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::record;

fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "metric-precision".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}

fn fixture() -> (tempfile::TempDir, Store) {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    let records = [
        record(
            "change",
            json!({"kind":"change","motivation":{"state":"recorded","reason":"user_requirement","problem":"Generic float scores omit the measurement's meaning","expected_behavior":"Preserve caller-defined values and calculations","rationale":"Retain exact values and calculation definitions for later audit","provenance":"contemporaneous"}}),
        ),
        record(
            "model",
            json!({"kind":"model","identity":"model","change_id":"change","label":"test-model","provider":"test","requested_model":"test"}),
        ),
        record(
            "suite",
            json!({"kind":"suite","identity":"suite","change_id":"change","cases":[],"aggregation":"caller-defined population"}),
        ),
        record(
            "target",
            json!({"kind":"target","identity":"target","change_id":"change","components":[{"id":"agent","role":"solve","model":{"assignment":"fixed","model_id":"model"}}]}),
        ),
        record(
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"external","runner_version":"1","state":"completed","selection_rule":"external cohort"}),
        ),
        record(
            "calculation-source",
            json!({"kind":"artifact","media_type":"text/plain","byte_size":120,"backings":[{"kind":"git_file","repository":"metric-fixtures","commit":"a".repeat(40),"object_format":"sha1","path":"evals/calculate.py","entry_mode":"100644"}]}),
        ),
    ];
    store
        .record_eval_bundle("project", &records, &actor())
        .unwrap();
    (temporary, store)
}

fn metric(id: &str, format: &str, parents: &[&str]) -> EvalRecordInput {
    record(
        id,
        json!({
            "kind":"metric","identity":"caller-metric","parents":parents,"change_id":"change",
            "name":"External measurement","description":"A caller-defined measurement, not an optimization command",
            "unit":"widgets/(calendar month · agent)","value_format":format,
            "calculation":{"description":"Apply the caller's saved calculation to its recorded input population","formula":"caller_defined(inputs)","source_artifacts":["calculation-source"],"population":"All attempts selected by the caller","aggregation":"Caller-defined; preserve raw observations","exclusions":"No implicit exclusions","missing_data":"Represent unavailability explicitly"},
            "interpretation":"Interpret according to this definition; Nexus does not decide what is better"
        }),
    )
}

fn measurement(id: &str, metric: &str, value: Value, replaces: Option<&str>) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"measurement","metric_id":metric,"subject":{"scope":"run","id":"run"},"value":value,"explanation":"Caller-reported observation","inputs":["run","calculation-source"],"evidence":["calculation-source"],"replaces":replaces}),
    )
}

#[test]
fn arbitrary_metric_formats_preserve_exact_text_through_storage_and_portable_transfer() {
    let (_temporary, mut store) = fixture();
    let examples = [
        (
            "integer",
            "decimal integer",
            "900719925474099312345678901234567890",
        ),
        (
            "decimal",
            "decimal",
            "7.00000000000000000000000000000000000000000000000000000000001",
        ),
        (
            "json",
            "application/json; caller schema v3",
            "{\n  \"count\": 9007199254740993, \"samples\": [0,2,5]\n}",
        ),
        (
            "csv",
            "text/csv; charset=utf-8",
            "case,label,weight\nalpha,accepted,0.000000000000000000001\n",
        ),
        ("category", "review-category/v2", "needs-human-review 🧪"),
        ("empty", "caller-defined empty sentinel", ""),
    ];
    for (id, format, text) in examples {
        store
            .record_eval_bundle(
                "project",
                &[
                    metric(id, format, &[]),
                    measurement(
                        &format!("value-{id}"),
                        id,
                        json!({"storage":"inline","text":text}),
                        None,
                    ),
                ],
                &actor(),
            )
            .unwrap();
    }
    let file = tempfile::NamedTempFile::new().unwrap();
    store.write_streamed_snapshot(file.as_file()).unwrap();
    let target_directory = tempfile::tempdir().unwrap();
    let mut target = Store::open(&target_directory.path().join("import.db")).unwrap();
    target.import_streamed_snapshot(file.path()).unwrap();
    for (id, format, text) in examples {
        let saved = serde_json::to_value(
            target
                .get_eval_record("project", &format!("value-{id}"))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["body"]["value"]["text"], text);
        let definition =
            serde_json::to_value(target.get_eval_record("project", id).unwrap().unwrap()).unwrap();
        assert_eq!(definition["body"]["value_format"], format);
        assert_eq!(
            definition["body"]["unit"],
            "widgets/(calendar month · agent)"
        );
    }
    let query: EvalQuery =
        serde_json::from_value(json!({"kind":"measurement","model":"test-model"})).unwrap();
    assert_eq!(
        target
            .list_eval_records("project", &query)
            .unwrap()
            .records
            .len(),
        examples.len()
    );
}

#[test]
fn metric_revision_and_remeasurement_history_preserve_old_values_and_motivations() {
    let (_temporary, mut store) = fixture();
    let mut revised_definition = metric(
        "definition-v2",
        "decimal milliseconds including tool wait",
        &["definition-v1"],
    );
    if let nexus::evals::EvalBody::Metric(value) = &mut revised_definition.body {
        value.revision.change_id = "definition-change".into();
    }
    let records = [
        metric("definition-v1", "decimal seconds", &[]),
        measurement(
            "original",
            "definition-v1",
            json!({"storage":"inline","text":"0.0000000000000000009"}),
            None,
        ),
        measurement(
            "corrected",
            "definition-v1",
            json!({"storage":"inline","text":"0.0000000000000000010"}),
            Some("original"),
        ),
        record(
            "definition-change",
            json!({"kind":"change","motivation":{"state":"recorded","reason":"observed_failure","problem":"Prior timing population excluded tool wait","expected_behavior":"Include the user's entire wait","rationale":"A faster model can still take longer when its tools wait","provenance":"contemporaneous"},"evidence":[{"kind":"record","id":"original"}]}),
        ),
        revised_definition,
        measurement(
            "new-definition-result",
            "definition-v2",
            json!({"storage":"unavailable","reason":"Required timing evidence was not captured"}),
            None,
        ),
        record(
            "comparison",
            json!({"kind":"comparison","baseline_ids":["run"],"candidate_ids":["run"],"grade_ids":[],"metric_ids":["definition-v1","definition-v2"],"measurement_ids":["original","new-definition-result"],"population":"Same recorded run","exclusions":"Missing evidence stays unavailable","validity":"disclosed_differences","differences":["Timing definition changed"],"result":"Caller reported observations under distinct definitions","uncertainty":"No numeric comparison without matching definitions"}),
        ),
    ];
    store
        .record_eval_bundle("project", &records, &actor())
        .unwrap();
    let original = serde_json::to_value(
        store
            .get_eval_record("project", "original")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(original["body"]["value"]["text"], "0.0000000000000000009");
    let why = store.why_eval_record("project", "comparison", 64).unwrap();
    for id in [
        "definition-v1",
        "definition-v2",
        "original",
        "corrected",
        "new-definition-result",
        "change",
        "definition-change",
        "calculation-source",
    ] {
        assert!(
            why.records.iter().any(|record| record.input.id == id),
            "missing {id}"
        );
    }
}
