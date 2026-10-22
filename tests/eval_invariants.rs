// @feature evals
// @spec docs/features/evals.md
// @boundary typed-eval-invariant-fixtures
use nexus::evals::{EvalProvenance, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::{json, Value};

fn record(id: &str, body: Value) -> EvalRecordInput {
    serde_json::from_value(json!({
        "id": id, "title": id, "occurred_at": "2026-10-22T12:00:00Z", "body": body
    }))
    .unwrap()
}

fn revision(kind: &str, identity: &str, fields: Value) -> Value {
    let mut body = json!({"kind":kind,"identity":identity,"parents":[],"change_id":"change"});
    body.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    body
}

fn provenance() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "invariants".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}

fn history() -> Vec<EvalRecordInput> {
    let mut records = vec![
        record(
            "change",
            json!({"kind":"change","motivation":{"state":"recorded","reason":"coverage_hypothesis","problem":"Need trustworthy associations","expected_behavior":"Retain consistent historical lineage","rationale":"Preserve an interpretable measurement","provenance":"contemporaneous"},"evidence":[]}),
        ),
        record(
            "case-a",
            revision("case", "case-a", json!({"expected_behavior":"A"})),
        ),
        record(
            "case-b",
            revision("case", "case-b", json!({"expected_behavior":"B"})),
        ),
        record(
            "model",
            revision(
                "model",
                "model",
                json!({"label":"test","provider":"test","requested_model":"test"}),
            ),
        ),
        record(
            "grader",
            revision(
                "grader",
                "grader",
                json!({"grading":{"method":"human","instructions":"Inspect outcome"},"metric":{"name":"pass","unit":"ratio","direction":"higher","minimum":0,"maximum":1,"threshold":1}}),
            ),
        ),
        record(
            "suite",
            revision(
                "suite",
                "suite",
                json!({"cases":[{"case_id":"case-a","grader_ids":["grader"],"weight":1,"split":"development"}],"aggregation":"mean"}),
            ),
        ),
        record(
            "target",
            revision(
                "target",
                "target",
                json!({"components":[{"id":"agent","role":"solve","model":{"assignment":"fixed","model_id":"model"}}]}),
            ),
        ),
        record(
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"fixture","runner_version":"1","state":"completed","selection_rule":"all","selected_cases":["case-a"]}),
        ),
    ];
    for (replicate, suffix) in [(0, "a"), (1, "b")] {
        records.push(record(&format!("trial-{suffix}"), json!({"kind":"trial","run_id":"run","case_id":"case-a","replicate":replicate,"state":"completed"})));
        records.push(record(&format!("attempt-{suffix}"), json!({"kind":"attempt","trial_id":format!("trial-{suffix}"),"state":"completed","started_at":"2026-10-22T12:00:00Z","finished_at":"2026-10-22T12:00:01Z"})));
    }
    records.extend([
        record("call-a", json!({"kind":"call","attempt_id":"attempt-a","component":"agent","instance":"agent-a","sequence":0,"model_id":"model"})),
        record("grade-a", json!({"kind":"grade","subject":{"scope":"attempt","id":"attempt-a"},"grader_id":"grader","pass_id":"original","state":"completed","score":0,"explanation":"Original grade"})),
        record("selection-a", json!({"kind":"selection","scope":"suite-production","revision_id":"suite","change_id":"change","effective_at":"2026-10-22T12:00:00Z"})),
    ]);
    records
}

fn reject(scenario: &str, records: &[EvalRecordInput]) {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    store
        .record_eval_bundle("project", &history(), &provenance())
        .unwrap();
    let result = store.record_eval_bundle("project", records, &provenance());
    assert!(
        result.is_err(),
        "{scenario}: invalid lineage was published: {result:?}"
    );
    for input in records {
        assert!(store
            .get_eval_record("project", &input.id)
            .unwrap()
            .is_none());
    }
}

fn reject_temporal_cycle(predecessor: &str, body: Value) {
    let records: Vec<_> = [("cycle-a", "cycle-b"), ("cycle-b", "cycle-a")]
        .into_iter()
        .map(|(id, parent)| {
            let mut body = body.clone();
            body[predecessor] = json!(parent);
            record(id, body)
        })
        .collect();
    reject(predecessor, &records);
}

macro_rules! temporal_cycle_tests {
    ($($name:ident => ($predecessor:literal, $body:expr)),+ $(,)?) => {
        $(#[test]
        fn $name() {
            reject_temporal_cycle($predecessor, $body);
        })+
    };
}

temporal_cycle_tests! {
    retry_predecessor_cycles_are_rejected => ("parent_id", json!({
        "kind":"attempt","trial_id":"trial-a","retry_reason":"Retry after interruption",
        "state":"completed","started_at":"2026-10-22T12:00:00Z","finished_at":"2026-10-22T12:00:01Z"
    })),
    call_predecessor_cycles_are_rejected => ("parent_id", json!({
        "kind":"call","attempt_id":"attempt-a","component":"agent","instance":"agent-a","sequence":1,"model_id":"model"
    })),
    regrade_predecessor_cycles_are_rejected => ("replaces", json!({
        "kind":"grade","subject":{"scope":"attempt","id":"attempt-a"},"grader_id":"grader",
        "pass_id":"corrected","state":"completed","score":1,"explanation":"Corrected assessment"
    })),
    selection_predecessor_cycles_are_rejected => ("previous_id", json!({
        "kind":"selection","scope":"suite-production","revision_id":"suite","change_id":"change",
        "effective_at":"2026-10-22T12:01:00Z"
    })),
    lifecycle_predecessor_cycles_are_rejected => ("previous_id", json!({
        "kind":"lifecycle","subject":{"scope":"run","id":"run"},"state":"completed"
    })),
}

#[test]
fn inconsistent_eval_lineage_is_rejected_atomically() {
    let scenarios = [
        (
            "trial case outside run suite",
            vec![record(
                "outside-trial",
                json!({"kind":"trial","run_id":"run","case_id":"case-b","replicate":0,"state":"completed"}),
            )],
        ),
        (
            "call parent in another attempt",
            vec![record(
                "call-b",
                json!({"kind":"call","attempt_id":"attempt-b","parent_id":"call-a","component":"agent","instance":"agent-b","sequence":1,"model_id":"model"}),
            )],
        ),
        (
            "regrading another subject",
            vec![record(
                "grade-b",
                json!({"kind":"grade","subject":{"scope":"attempt","id":"attempt-b"},"grader_id":"grader","pass_id":"corrected","state":"completed","score":1,"explanation":"Different attempt","replaces":"grade-a"}),
            )],
        ),
        (
            "selection previous in another scope",
            vec![record(
                "selection-b",
                json!({"kind":"selection","scope":"target-production","revision_id":"target","previous_id":"selection-a","change_id":"change","effective_at":"2026-10-22T12:01:00Z"}),
            )],
        ),
        (
            "revision parent cycle",
            vec![
                record(
                    "cycle-a",
                    revision(
                        "case",
                        "cycle",
                        json!({"parents":["cycle-b"],"expected_behavior":"Cycle fixture"}),
                    ),
                ),
                record(
                    "cycle-b",
                    revision(
                        "case",
                        "cycle",
                        json!({"parents":["cycle-a"],"expected_behavior":"Cycle fixture"}),
                    ),
                ),
            ],
        ),
        (
            "retry parent in another trial",
            vec![record(
                "retry-b",
                json!({"kind":"attempt","trial_id":"trial-b","parent_id":"attempt-a","retry_reason":"Retry after interruption","state":"completed","started_at":"2026-10-22T12:00:00Z","finished_at":"2026-10-22T12:00:01Z"}),
            )],
        ),
    ];
    for (scenario, records) in scenarios {
        reject(scenario, &records);
    }
}

#[test]
fn consistent_lineage_allows_new_graders_retries_and_shared_revision_ancestors() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    store
        .record_eval_bundle("project", &history(), &provenance())
        .unwrap();
    let mut grader = history()
        .into_iter()
        .find(|record| record.id == "grader")
        .unwrap();
    grader.id = "grader-v2".into();
    if let nexus::evals::EvalBody::Grader(body) = &mut grader.body {
        body.revision.parents = vec!["grader".into()];
    }
    let mut records = vec![
        grader,
        record(
            "retry-a",
            json!({"kind":"attempt","trial_id":"trial-a","parent_id":"attempt-a","retry_reason":"Retry after interruption","state":"completed","started_at":"2026-10-22T12:00:00Z","finished_at":"2026-10-22T12:00:01Z"}),
        ),
        record(
            "call-child",
            json!({"kind":"call","attempt_id":"attempt-a","parent_id":"call-a","component":"agent","instance":"agent-a","sequence":1,"model_id":"model"}),
        ),
        record(
            "grade-corrected",
            json!({"kind":"grade","subject":{"scope":"attempt","id":"attempt-a"},"grader_id":"grader-v2","pass_id":"corrected","state":"completed","score":1,"explanation":"Corrected rubric","replaces":"grade-a"}),
        ),
        record(
            "selection-next",
            json!({"kind":"selection","scope":"suite-production","revision_id":"suite","previous_id":"selection-a","change_id":"change","effective_at":"2026-10-22T12:01:00Z"}),
        ),
    ];
    for (id, parents) in [
        ("revision-a", vec![]),
        ("revision-b", vec!["revision-a"]),
        ("revision-c", vec!["revision-a"]),
        ("revision-d", vec!["revision-b", "revision-c"]),
    ] {
        records.push(record(
            id,
            revision(
                "case",
                "fork",
                json!({"parents":parents,"expected_behavior":"Retain a valid revision merge"}),
            ),
        ));
    }
    records.reverse();
    assert_eq!(
        store
            .record_eval_bundle("project", &records, &provenance())
            .unwrap()
            .inserted,
        records.len()
    );
}
