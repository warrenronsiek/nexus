// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;
use serde_json::json;

#[cfg(feature = "sqlite")]
#[test]
fn initial_category_catalog_is_published_atomically() {
    use diesel::connection::InstrumentationEvent;
    use std::sync::mpsc;
    use std::time::Duration;

    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("analytics.db");
    let mut writer = Store::open(&path).unwrap();
    let mut reader = Store::open(&path).unwrap();
    let (written, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut written = Some(written);
    writer
        .connection
        .set_instrumentation(move |event: InstrumentationEvent<'_>| {
            if let InstrumentationEvent::FinishQuery {
                query, error: None, ..
            } = event
            {
                let query = query.to_string();
                if query.starts_with("INSERT INTO") && query.contains("analytics_categories") {
                    if let Some(written) = written.take() {
                        written.send(()).unwrap();
                        released.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                }
            }
        });
    let seed = std::thread::spawn(move || writer.ensure_activity_categories());
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    let visible = analytics_categories::table
        .count()
        .get_result::<i64>(&mut reader.connection);
    release.send(()).unwrap();
    seed.join().unwrap().unwrap();
    assert_eq!(
        visible.unwrap(),
        0,
        "a partial category catalog was visible"
    );
    assert_eq!(
        reader.activity_categories().unwrap().len(),
        crate::analytics::capture::known_categories().len()
    );
}

#[test]
fn only_one_classification_attempt_is_reserved_for_a_repeated_pattern_each_day() {
    let mut store = Store::open_memory().unwrap();
    for (id, granted) in [("first", true), ("second", false)] {
        let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","agent":"pi","tool_use_id":id,"tool_name":"custom","tool_input":{}})).unwrap();
        let action = Action {
            name: "custom".into(),
            kind: ResourceKind::Tool,
            origin: Origin::HostTool,
            category_id: "unclassified".into(),
            description: "Unknown purpose".into(),
            pattern_key: "a".repeat(64),
            model_input: None,
        };
        store
            .record_activity_call("p", &input, &[action], "succeeded")
            .unwrap();
        let job = store
            .resolve_classification_target(
                "p",
                &input.context,
                &ClassificationTarget::Action {
                    invocation_id: id.into(),
                    index: 0,
                    pattern_key: "a".repeat(64),
                },
            )
            .unwrap();
        assert_eq!(
            store.reserve_activity_classification(&job, 10).unwrap(),
            granted
        );
    }
}

#[test]
fn a_classification_report_requires_a_matching_reserved_attempt() {
    let mut store = Store::open_memory().unwrap();
    let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","agent":"pi","tool_use_id":"t","tool_name":"custom","tool_input":{}})).unwrap();
    let action = Action {
        name: "custom".into(),
        kind: ResourceKind::Tool,
        origin: Origin::HostTool,
        category_id: "unclassified".into(),
        description: "Unknown purpose".into(),
        pattern_key: "a".repeat(64),
        model_input: None,
    };
    store
        .record_activity_call("p", &input, &[action], "succeeded")
        .unwrap();
    let job = store
        .resolve_classification_target(
            "p",
            &input.context,
            &ClassificationTarget::Action {
                invocation_id: "t".into(),
                index: 0,
                pattern_key: "a".repeat(64),
            },
        )
        .unwrap();
    let draft = CategoryDraft {
        category_id: "ci.inspect".into(),
        name: "Inspect CI".into(),
        definition: "Inspect check status".into(),
        description: "Inspect CI checks".into(),
    };
    assert!(store
        .apply_activity_classification(&job, &draft, "test")
        .is_err());
}

#[test]
fn taxonomy_rename_and_merge_preserve_an_audit_history() {
    let mut store = Store::open_memory().unwrap();
    store.ensure_activity_categories().unwrap();
    store
        .rename_activity_category(
            "ci.inspect",
            "Inspect CI",
            "Inspect continuous integration checks",
        )
        .unwrap();
    store
        .merge_activity_categories("ci.inspect", "vcs.inspect")
        .unwrap();
    let revisions = analytics_revisions::table
        .filter(analytics_revisions::evidence.eq("manual"))
        .count()
        .get_result::<i64>(&mut store.connection)
        .unwrap();
    assert_eq!(revisions, 2);
}

#[test]
fn classifying_a_tool_definition_enriches_its_previously_unclassified_invocations() {
    let mut store = Store::open_memory().unwrap();
    let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","agent":"pi","tool_use_id":"t","tool_name":"checks","tool_input":{}})).unwrap();
    let resource = Resource {
        name: "checks".into(),
        kind: ResourceKind::Tool,
        origin: Origin::HostTool,
        exposure: Exposure::Exposed,
        description: "Inspect CI checks".into(),
        activities: vec![],
        version: Some("v1".into()),
    };
    store
        .record_inventory(
            "p",
            &input.context,
            "host_tools",
            std::slice::from_ref(&resource),
            true,
        )
        .unwrap();
    store
        .record_activity_call(
            "p",
            &input,
            &crate::analytics::capture::resolve(&input, &[]),
            "succeeded",
        )
        .unwrap();
    let job = store
        .resolve_classification_target(
            "p",
            &input.context,
            &ClassificationTarget::Resource {
                name: resource.name.clone(),
                resource_kind: resource.kind,
                pattern_key: resource_pattern(&resource),
            },
        )
        .unwrap();
    assert!(store.reserve_activity_classification(&job, 10).unwrap());
    store
        .apply_activity_classification(
            &job,
            &CategoryDraft {
                category_id: "ci.inspect".into(),
                name: "Inspect CI".into(),
                definition: "Inspect continuous integration checks".into(),
                description: "Inspect CI checks".into(),
            },
            "test",
        )
        .unwrap();
    assert_eq!(
        store
            .activity_summary(Some("p"))
            .unwrap()
            .classified_actions,
        1
    );
    assert_eq!(
        store
            .activity_examples(Some("p"), Some("ci.inspect"), None, 0)
            .unwrap()
            .examples[0]
            .evidence,
        "definition"
    );
}

#[test]
fn a_changed_tool_definition_does_not_relabel_calls_from_an_older_definition() {
    let mut store = Store::open_memory().unwrap();
    let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","agent":"pi","tool_use_id":"old","tool_name":"checks","tool_input":{}})).unwrap();
    let mut resource = Resource {
        name: "checks".into(),
        kind: ResourceKind::Tool,
        origin: Origin::HostTool,
        exposure: Exposure::Exposed,
        description: "Inspect CI checks".into(),
        activities: vec![],
        version: Some("v1".into()),
    };
    store
        .record_inventory(
            "p",
            &input.context,
            "host_tools",
            std::slice::from_ref(&resource),
            true,
        )
        .unwrap();
    store
        .record_activity_call(
            "p",
            &input,
            &crate::analytics::capture::resolve(&input, &[]),
            "succeeded",
        )
        .unwrap();
    resource.version = Some("v2".into());
    resource.description = "Modify issues".into();
    store
        .record_inventory(
            "p",
            &input.context,
            "host_tools",
            std::slice::from_ref(&resource),
            true,
        )
        .unwrap();
    let job = store
        .resolve_classification_target(
            "p",
            &input.context,
            &ClassificationTarget::Resource {
                name: resource.name.clone(),
                resource_kind: resource.kind,
                pattern_key: resource_pattern(&resource),
            },
        )
        .unwrap();
    assert!(store.reserve_activity_classification(&job, 10).unwrap());
    store
        .apply_activity_classification(
            &job,
            &CategoryDraft {
                category_id: "issues.change".into(),
                name: "Modify issues".into(),
                definition: "Modify issue records".into(),
                description: "Modify issues".into(),
            },
            "test",
        )
        .unwrap();
    assert_eq!(
        store
            .activity_summary(Some("p"))
            .unwrap()
            .classified_actions,
        0
    );
}
