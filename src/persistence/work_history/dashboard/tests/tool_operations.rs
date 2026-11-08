// @feature work-history
// @feature usage-analytics
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/persistence.md
use super::*;

#[test]
fn historical_native_tool_reuses_exact_invocation_operations_and_return_semantics() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let id = stable_work_id(&["fixture", "host", "search"]);
    record(&mut store, &project, native_events(&id));
    let input = serde_json::from_value(json!({"session_id":"host","agent":"fixture","tool_use_id":"search","tool_name":"Bash","tool_input":{"command":"rg private_literal private_path"}})).unwrap();
    let actions = crate::analytics::capture::resolve(&input, &[]);
    store
        .record_activity_call(&project, &input, &actions, "succeeded")
        .unwrap();
    let WorkResponse::Detail {
        record: Some(detail),
    } = store
        .work_command(
            &project,
            &WorkActor {
                agent: "fixture".into(),
                session_id: "host".into(),
                model_id: None,
                config_hash: "fixture".into(),
            },
            &WorkAction::Get {
                project_id: None,
                subject: WorkSubject::Activity { id: id.clone() },
            },
        )
        .unwrap()
    else {
        panic!("detail")
    };
    let node = WorkIntervalNode::from(&detail.summary);
    let value = serde_json::to_value(&node).unwrap();
    assert_eq!(value["operations"][0]["name"], "rg");
    assert_eq!(value["operations"][0]["category_id"], "code.search");
    assert_eq!(value["operations"][0]["evidence"], "deterministic");
    assert_eq!(node.title, "rg");
    assert_eq!(value["call_mode"], "synchronous");
    assert!(node.returned_at.is_some());
    assert!(!serde_json::to_string(&node)
        .unwrap()
        .contains("private_literal"));
}

fn native_events(id: &str) -> Vec<WorkObservationInput> {
    let mut events = vec![
        observation(
            "root",
            "session",
            "root",
            json!({"event":"session_opened","title":"Root","context":{}}),
        ),
        observation(
            "declare",
            "activity",
            id,
            json!({"event":"activity_declared","name":"Bash","kind":"tool","work_session_id":"root","context":{"host_session_id":"host","labels":{"nexus.accounting.role":"execution"}}}),
        ),
        observation("start", "activity", id, json!({"event":"started"})),
        observation(
            "finish",
            "activity",
            id,
            json!({"event":"finished","outcome":"succeeded"}),
        ),
    ];
    for event in &mut events {
        event.producer.name = "nexus-companion".into();
        event.method = "companion_callback_wall_clock".into();
    }
    events[3].occurred_at += chrono::Duration::seconds(2);
    events
}

#[test]
fn completed_analytics_never_fills_a_missing_source_finish_or_another_invocation() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let id = stable_work_id(&["fixture", "host", "search"]);
    let mut events = native_events(&id);
    events.pop();
    record(&mut store, &project, events);
    let input = serde_json::from_value(json!({"session_id":"host","agent":"fixture","tool_use_id":"different","tool_name":"Bash","tool_input":{"command":"rg private_literal"}})).unwrap();
    let actions = crate::analytics::capture::resolve(&input, &[]);
    store
        .record_activity_call(&project, &input, &actions, "succeeded")
        .unwrap();
    let detail = projection::Projection::new(&mut store.connection, &project)
        .detail(&WorkSubject::Activity { id: id.clone() })
        .unwrap()
        .unwrap();
    assert!(detail.summary.operations.is_empty());
    assert_eq!(detail.summary.title, "Bash");
    assert!(detail.summary.finished_at.is_none());
    assert!(WorkIntervalNode::from(&detail.summary)
        .returned_at
        .is_none());
    let mut matching = input;
    matching.tool_use_id = "search".into();
    store
        .record_activity_call(&project, &matching, &actions, "succeeded")
        .unwrap();
    let detail = projection::Projection::new(&mut store.connection, &project)
        .detail(&WorkSubject::Activity { id: id.clone() })
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.operations[0].name, "rg");
    assert!(detail.summary.finished_at.is_none());
    assert!(WorkIntervalNode::from(&detail.summary)
        .returned_at
        .is_none());
    let other = uuid::Uuid::new_v4().to_string();
    record(&mut store, &other, native_events(&id));
    let detail = projection::Projection::new(&mut store.connection, &other)
        .detail(&WorkSubject::Activity { id })
        .unwrap()
        .unwrap();
    assert!(detail.summary.operations.is_empty());
}

#[test]
fn caller_authored_tool_kind_does_not_imply_synchronous_invocation() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let mut events = native_events("custom");
    events[1].producer.name = "caller".into();
    events[1].method = "caller".into();
    record(&mut store, &project, events);
    let detail = projection::Projection::new(&mut store.connection, &project)
        .detail(&WorkSubject::Activity {
            id: "custom".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        WorkIntervalNode::from(&detail.summary).call_mode,
        WorkCallMode::Unknown
    );
}

#[test]
fn compact_timeline_bounds_operation_detail_without_losing_its_total() {
    let mut summary = WorkSummary::unobserved(&WorkSubject::Activity { id: "tool".into() });
    summary.operations = (0..8)
        .map(|i| WorkToolOperation {
            name: format!("operation-{i}"),
            category_id: "code.search".into(),
            category_name: "code search".into(),
            description: "Search source".into(),
            evidence: "deterministic".into(),
        })
        .collect();
    let interval = WorkIntervalNode::from(&summary);
    assert_eq!(interval.operations.len(), 3);
    assert!(interval
        .gaps
        .iter()
        .any(|gap| gap == "tool_operations_truncated"));
    assert_eq!(summary.operations.len(), 8);
}

#[test]
fn durable_classification_survives_a_missing_analytics_write() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let mut events = native_events("durable");
    events.push(observation("classification", "activity", "durable", json!({"event":"milestone","name":"Tool classification observed","context":{"labels":{"nexus.tool.operation.0":"rg","nexus.tool.category.0":"code.search"}}})));
    record(&mut store, &project, events);
    let detail = projection::Projection::new(&mut store.connection, &project)
        .detail(&WorkSubject::Activity {
            id: "durable".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(detail.summary.title, "rg");
    assert_eq!(detail.summary.operations[0].category_id, "code.search");
    assert_eq!(detail.summary.operations[0].evidence, "deterministic");
}

#[test]
fn operation_enrichment_preserves_a_moderate_session_in_the_primary_timeline_budget() {
    let mut view = DashboardProjection::new(Some("root"), false, WorkSessionOrigin::All);
    for index in 0..481 {
        let mut summary = WorkSummary::unobserved(&WorkSubject::Activity {
            id: format!("{index:064}"),
        });
        summary.title = "rg; sed; cat".into();
        summary.kind = "tool".into();
        summary.parent_activity_id = Some("a".repeat(64));
        summary.executor = Some("e".repeat(64));
        summary.started_at = Some("2026-11-07T12:00:00Z".parse().unwrap());
        summary.finished_at = Some("2026-11-07T12:00:01Z".parse().unwrap());
        summary.operations = ["rg", "sed", "cat"]
            .into_iter()
            .map(|name| WorkToolOperation {
                name: name.into(),
                category_id: "code.search".into(),
                category_name: "code search".into(),
                description: "Search source".into(),
                evidence: "deterministic".into(),
            })
            .collect();
        view.view.intervals.push(WorkIntervalNode::from(&summary));
    }
    view.bound_output().unwrap();
    assert_eq!(view.view.intervals.len(), 481);
}
