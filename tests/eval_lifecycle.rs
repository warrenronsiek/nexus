// @feature evals
// @spec docs/features/evals.md
// @boundary typed-eval-lifecycle-fixtures
use nexus::evals::{EvalProvenance, EvalQuery, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::json;
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::record;

fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "lifecycle".into(),
        model_id: None,
        config_hash: "fixture".into(),
    }
}

fn initial_history() -> Vec<EvalRecordInput> {
    vec![
        record(
            "change",
            json!({"kind":"change","motivation":{"state":"missing","reason":"Synthetic lifecycle fixture"}}),
        ),
        record(
            "case",
            json!({"kind":"case","identity":"case","change_id":"change","expected_behavior":"Retain the observed outcome"}),
        ),
        record(
            "model",
            json!({"kind":"model","identity":"model","change_id":"change","label":"test","provider":"test","requested_model":"test"}),
        ),
        record(
            "suite",
            json!({"kind":"suite","identity":"suite","change_id":"change","cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"mean"}),
        ),
        record(
            "target",
            json!({"kind":"target","identity":"target","change_id":"change","components":[{"id":"agent","role":"solve","model":{"assignment":"fixed","model_id":"model"}}]}),
        ),
        record(
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"planned","selection_rule":"all"}),
        ),
        record(
            "trial",
            json!({"kind":"trial","run_id":"run","case_id":"case","replicate":0,"state":"planned"}),
        ),
        record(
            "attempt",
            json!({"kind":"attempt","trial_id":"trial","state":"running","started_at":"2026-10-22T12:00:00Z"}),
        ),
    ]
}

fn event(
    id: &str,
    scope: &str,
    subject: &str,
    state: &str,
    previous: Option<&str>,
) -> EvalRecordInput {
    record(
        id,
        json!({"kind":"lifecycle","subject":{"scope":scope,"id":subject},"state":state,"previous_id":previous}),
    )
}

fn fixture() -> (tempfile::TempDir, Store) {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("history.db")).unwrap();
    store
        .record_eval_bundle("project", &initial_history(), &actor())
        .unwrap();
    (temporary, store)
}

#[test]
fn lifecycle_reports_finish_online_executions_without_replacing_original_records() {
    let (_temporary, mut store) = fixture();
    let reports = vec![
        event("run-running", "run", "run", "running", None),
        event(
            "run-completed",
            "run",
            "run",
            "completed",
            Some("run-running"),
        ),
        event("trial-completed", "trial", "trial", "completed", None),
        event(
            "attempt-interrupted",
            "attempt",
            "attempt",
            "interrupted",
            None,
        ),
    ];
    store
        .record_eval_bundle("project", &reports, &actor())
        .unwrap();
    for (subject, initial, report) in [
        ("run", "planned", "run-completed"),
        ("trial", "planned", "trial-completed"),
        ("attempt", "running", "attempt-interrupted"),
    ] {
        let original =
            serde_json::to_value(store.get_eval_record("project", subject).unwrap().unwrap())
                .unwrap();
        assert_eq!(original["body"]["state"], initial);
        assert!(store
            .why_eval_record("project", subject, 64)
            .unwrap()
            .records
            .iter()
            .any(|record| record.input.id == report));
    }
    let page = store
        .list_eval_records(
            "project",
            &EvalQuery {
                kind: Some(serde_json::from_value(json!("lifecycle")).unwrap()),
                model: Some("test".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.records.len(), reports.len());
}

#[test]
fn lifecycle_predecessors_must_report_the_same_execution_subject() {
    let (_temporary, mut store) = fixture();
    store
        .record_eval_bundle(
            "project",
            &[event("run-running", "run", "run", "running", None)],
            &actor(),
        )
        .unwrap();
    let result = store.record_eval_bundle(
        "project",
        &[event(
            "wrong-parent",
            "attempt",
            "attempt",
            "completed",
            Some("run-running"),
        )],
        &actor(),
    );
    assert!(
        result.is_err(),
        "lifecycle histories must not join different executions"
    );
    assert!(store
        .get_eval_record("project", "wrong-parent")
        .unwrap()
        .is_none());
}

#[test]
fn ordinary_reads_expose_current_lifecycle_frontiers_without_picking_a_fork() {
    let (_temporary, mut store) = fixture();
    let reports = [
        event("run-running", "run", "run", "running", None),
        event(
            "run-completed",
            "run",
            "run",
            "completed",
            Some("run-running"),
        ),
    ];
    store
        .record_eval_bundle("project", &reports, &actor())
        .unwrap();
    let before =
        serde_json::to_value(store.get_eval_record("project", "run").unwrap().unwrap()).unwrap();
    assert_eq!(before["lifecycle"]["initial_state"], "planned");
    assert_eq!(
        before["lifecycle"]["frontiers"].as_array().unwrap().len(),
        1
    );
    assert_eq!(before["lifecycle"]["frontiers"][0]["id"], "run-completed");
    assert_eq!(before["lifecycle"]["frontiers"][0]["state"], "completed");
    store
        .record_eval_bundle(
            "project",
            &[event(
                "run-interrupted",
                "run",
                "run",
                "interrupted",
                Some("run-running"),
            )],
            &actor(),
        )
        .unwrap();
    let after =
        serde_json::to_value(store.get_eval_record("project", "run").unwrap().unwrap()).unwrap();
    let states: std::collections::BTreeSet<_> = after["lifecycle"]["frontiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|report| report["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        states,
        std::collections::BTreeSet::from(["completed", "interrupted"])
    );
    assert_eq!(after["lifecycle"]["truncated"], false);
    assert_eq!(after["body"]["state"], "planned");
}

#[test]
fn lifecycle_projection_discloses_a_bounded_set_of_competing_reports() {
    let (_temporary, mut store) = fixture();
    let reports: Vec<_> = (0..65)
        .map(|index| event(&format!("report-{index:03}"), "run", "run", "running", None))
        .collect();
    for batch in reports.chunks(64) {
        store
            .record_eval_bundle("project", batch, &actor())
            .unwrap();
    }
    let detail =
        serde_json::to_value(store.get_eval_record("project", "run").unwrap().unwrap()).unwrap();
    assert_eq!(
        detail["lifecycle"]["frontiers"].as_array().unwrap().len(),
        64
    );
    assert_eq!(detail["lifecycle"]["truncated"], true);
}
