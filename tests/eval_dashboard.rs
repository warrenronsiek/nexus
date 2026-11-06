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

#[test]
fn partial_dimension_prefetch_keeps_each_runs_full_model_composition() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("eval.db")).unwrap();
    let mut definitions = definitions();
    definitions.retain(|record| record.id != "run");
    let components: Vec<_> = (0..20).map(|i| json!({"id":format!("worker-{i}"),"role":"answer","repository_id":"repo","model":{"assignment":"fixed","model_id":format!("model-{i}")}})).collect();
    *definitions
        .iter_mut()
        .find(|record| record.id == "target")
        .unwrap() = revision("target", "target", json!({"components":components}));
    definitions.extend((0..20).map(|i| revision(&format!("model-{i}"), "model", json!({"label":format!("model-{i}"),"provider":"fixture","requested_model":format!("model-{i}")}))));
    store
        .record_eval_bundle("p", &definitions, &actor())
        .unwrap();
    let runs: Vec<_> = (0..64).map(|i| record(&format!("run-{i:02}"),json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"planned","selection_rule":"all"}))).collect();
    store.record_eval_bundle("p", &runs, &actor()).unwrap();
    let dashboard = store.eval_dashboard("p", None).unwrap();
    assert_eq!(dashboard.run_count, 64);
    assert_eq!(dashboard.groups.len(), 1);
    assert_eq!(dashboard.groups[0].models.len(), 20);
    assert_eq!(dashboard.groups[0].repository, vec!["router"]);
    assert_eq!(dashboard.groups[0].statuses.get("planned"), Some(&64));
    assert!(!dashboard.truncated);
}

fn titled(id: &str, title: &str, kind: &str, identity: &str, fields: Value) -> EvalRecordInput {
    let mut body = json!({"kind":kind,"identity":identity,"change_id":"change"});
    body.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    serde_json::from_value(
        json!({"id":id,"title":title,"occurred_at":"2026-10-22T12:00:00Z","body":body}),
    )
    .unwrap()
}
fn agent_records(prefix: &str, agent: Option<&str>) -> Vec<EvalRecordInput> {
    let mut target = json!({"components":[{"id":prefix,"role":"answer","repository_id":"repo","model":{"assignment":"fixed","model_id":"model"}}]});
    if let Some(agent) = agent {
        target["agent"] = json!(agent);
    }
    vec![
        titled(
            &format!("{prefix}-suite"),
            "suite/part-0",
            "suite",
            &format!("{prefix}/automatic-suite/part-0"),
            json!({"cases":[{"case_id":"case","grader_ids":[],"weight":1,"split":"development"}],"aggregation":"caller"}),
        ),
        titled(
            &format!("{prefix}-target"),
            "variant; prompt source sha256 0",
            "target",
            &format!("{prefix}/automatic-target"),
            target,
        ),
        record(
            &format!("{prefix}-run"),
            json!({"kind":"run","target_id":format!("{prefix}-target"),"suite_id":format!("{prefix}-suite"),"runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"}),
        ),
        record(
            &format!("{prefix}-latency"),
            json!({"kind":"measurement","metric_id":"metric","subject":{"scope":"run","id":format!("{prefix}-run")},"value":{"storage":"inline","text":"3.5"},"explanation":"Caller result","replaces":null}),
        ),
    ]
}
#[test]
fn dashboard_groups_name_their_agent_and_distinguish_generic_suite_titles() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("agents.db")).unwrap();
    let mut records = definitions();
    records.retain(|record| record.id != "run");
    records.extend(agent_records("router", Some("Source router")));
    records.extend(agent_records("briefing", None));
    store.record_eval_bundle("p", &records, &actor()).unwrap();

    let dashboard = serde_json::to_value(store.eval_dashboard("p", None).unwrap()).unwrap();
    let mut groups = dashboard["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            (
                group["agent"].as_str().unwrap().to_owned(),
                group["agent_declared"].as_bool().unwrap(),
                group["label"].as_str().unwrap().to_owned(),
                group["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    groups.sort();
    assert_eq!(groups.len(), 2, "{dashboard}");
    assert_eq!(
        (groups[0].0.as_str(), groups[0].1, groups[0].2.as_str()),
        (
            "Source router",
            true,
            "Source router · router/automatic-suite/part-0 · test-model"
        )
    );
    assert_eq!(
        (groups[1].0.as_str(), groups[1].1, groups[1].2.as_str()),
        (
            "briefing",
            false,
            "briefing · automatic-suite/part-0 · test-model"
        )
    );

    let selected =
        serde_json::to_value(store.eval_dashboard("p", Some(&groups[0].3)).unwrap()).unwrap();
    let point = &selected["series"][0]["points"][0];
    assert_eq!(point["occurred_at"], "2026-10-22T12:00:00Z", "{selected}");
    assert_eq!(point["numeric_value"], 3.5, "{selected}");
}
#[test]
fn target_agent_names_are_validated() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("invalid.db")).unwrap();
    store
        .record_eval_bundle("p", &definitions(), &actor())
        .unwrap();
    for agent in [" ", "line\nbreak"] {
        let mut records = agent_records("invalid", Some(agent));
        records.truncate(2);
        assert!(
            store.record_eval_bundle("p", &records, &actor()).is_err(),
            "accepted agent {agent:?}"
        );
    }
}
