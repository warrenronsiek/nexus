// @feature persistence
// @feature observability-ui
// @spec docs/features/persistence.md
// @spec docs/features/observability-ui.md
// @boundary typed-fixture-json: valid caller-owned records create projects without coordination hosts.
use super::*;
use crate::evals::{EvalProvenance, EvalRecordInput};
use crate::work_history::{WorkAction, WorkActor, WorkObservationInput};
use serde_json::json;

fn seed_work(store: &mut Store, project: &str) {
    let observation: WorkObservationInput = serde_json::from_value(json!({
        "id":"opened", "subject":{"scope":"session","id":"logical-root"},
        "occurred_at":"2026-11-01T12:00:00Z", "producer":{"name":"fixture","version":"1","instance_id":"test"},
        "method":"caller", "body":{"event":"session_opened","title":"Independent work","context":{}}
    })).unwrap();
    let actor = WorkActor {
        agent: "fixture".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    store
        .work_command(
            project,
            &actor,
            &WorkAction::Record {
                observations: vec![observation],
            },
        )
        .unwrap();
}
fn seed_eval(store: &mut Store, project: &str) {
    let input: EvalRecordInput = serde_json::from_value(json!({
        "id":"change", "title":"Caller-owned evaluation", "occurred_at":"2026-11-01T12:00:00Z",
        "body":{"kind":"change","motivation":{"state":"missing","reason":"Historical evidence fixture"}}
    })).unwrap();
    let actor = EvalProvenance {
        agent: "fixture".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    store.record_eval_bundle(project, &[input], &actor).unwrap();
}

#[test]
fn project_discovery_includes_work_and_evals_without_coordination_sessions() {
    let mut store = Store::open_memory().unwrap();
    seed_work(&mut store, "work-only");
    seed_eval(&mut store, "eval-only");
    let window = store.list_projects_window().unwrap();
    let ids: std::collections::BTreeSet<_> = window
        .items
        .iter()
        .map(|project| project.project_id.as_str())
        .collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from(["eval-only", "work-only"])
    );
    assert!(!window.truncated);
    assert!(window
        .items
        .iter()
        .all(|project| project.agents.is_empty() && project.worktrees.is_empty()));
}

fn actor<T: serde::de::DeserializeOwned>() -> T {
    serde_json::from_value(
        json!({"agent":"fixture","session_id":"caller","model_id":null,"config_hash":"fixture"}),
    )
    .unwrap()
}

fn seed_reporting_projects(store: &mut Store) {
    store
        .report_papercut(
            "papercuts-only",
            "Development friction",
            "A recorded problem",
            &actor(),
        )
        .unwrap();
    let draft = serde_json::from_value(json!({"message":"An unusual input","type":"input","environment":"test","version":"1","model":"fixture","trace":"source","metadata":{}})).unwrap();
    store
        .record_sos_report("sos-only", draft, &actor())
        .unwrap();
    let observation = serde_json::from_value(json!({"id":"concern-report","concern_id":"problem","observed_at":"2026-11-01T12:00:00Z","context":{},"body":{"event":"reported","title":"A code problem","description":"The symptom recurred","encounter":{}}})).unwrap();
    store
        .concern_command(
            "concerns-only",
            &actor(),
            &crate::code_concerns::ConcernAction::Record {
                observations: vec![observation],
            },
        )
        .unwrap();
}

fn seed_usage_project(store: &mut Store) {
    use crate::persistence::{
        CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilitySource,
    };
    let observation = CapabilityObservation {
        project_id: "usage-only".into(),
        agent: "fixture".into(),
        session_id: "caller".into(),
        turn_id: None,
        invocation_id: "call".into(),
        parent_invocation_id: None,
        kind: CapabilityKind::Tool,
        name: "supplied-tool".into(),
        source: CapabilitySource::HostHook,
        evidence: CapabilityEvidence::NativeHook,
        config_hash: "fixture".into(),
        model_id: None,
    };
    store.observe_capability_start(&observation).unwrap();
    let context = serde_json::from_value(json!({"agent":"fixture","session_id":"caller"})).unwrap();
    store
        .record_inventory("inventory-only", &context, "tools", &[], true)
        .unwrap();
}

#[test]
fn project_discovery_includes_reporting_memory_and_inventory_without_host_sessions() {
    let mut store = Store::open_memory().unwrap();
    seed_reporting_projects(&mut store);
    seed_usage_project(&mut store);
    store
        .add_memory(
            &crate::memory::MemoryScope::project("memory-only"),
            "A project note",
            &actor(),
        )
        .unwrap();
    store
        .add_memory(
            &crate::memory::MemoryScope::Global,
            "A global note",
            &actor(),
        )
        .unwrap();
    let window = store.list_projects_window().unwrap();
    let ids: std::collections::BTreeSet<_> = window
        .items
        .iter()
        .map(|project| project.project_id.as_str())
        .collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([
            "concerns-only",
            "inventory-only",
            "memory-only",
            "papercuts-only",
            "sos-only",
            "usage-only"
        ])
    );
    assert!(!window.truncated);
}

#[test]
fn project_discovery_merges_duplicate_sources_before_recency_order_and_limit() {
    let mut store = Store::open_memory().unwrap();
    for index in 0..18 {
        let project = format!("project-{index:02}");
        seed_work(&mut store, &project);
        seed_eval(&mut store, &project);
        let timestamp = format!("2026-11-01T12:00:{index:02}+00:00");
        set_receipt_times(&mut store, &project, &timestamp);
    }
    store
        .touch_session(
            "project-00",
            "host",
            "codex",
            Some("/recorded/worktree"),
            "fixture",
        )
        .unwrap();
    diesel::update(sessions::table.filter(sessions::project_id.eq("project-00")))
        .set(sessions::last_seen_at.eq("2026-11-02T12:00:00+00:00"))
        .execute(&mut store.connection)
        .unwrap();
    let window = store.list_projects_window().unwrap();
    assert!(window.truncated);
    assert_eq!(window.items.len(), 16);
    assert_eq!(window.items[0].project_id, "project-00");
    assert_eq!(window.items[0].agents, vec!["codex"]);
    assert_eq!(window.items[0].worktrees, vec!["/recorded/worktree"]);
    assert_eq!(window.items[1].project_id, "project-17");
    let ids: std::collections::BTreeSet<_> = window
        .items
        .iter()
        .map(|project| &project.project_id)
        .collect();
    assert_eq!(ids.len(), 16, "feature sources duplicated a project");
}

fn set_receipt_times(store: &mut Store, project: &str, timestamp: &str) {
    use crate::persistence::schema::{eval_records, work_observations};
    diesel::update(work_observations::table.filter(work_observations::project_id.eq(project)))
        .set(work_observations::recorded_at.eq(timestamp))
        .execute(&mut store.connection)
        .unwrap();
    diesel::update(eval_records::table.filter(eval_records::project_id.eq(project)))
        .set(eval_records::recorded_at.eq(timestamp))
        .execute(&mut store.connection)
        .unwrap();
}
