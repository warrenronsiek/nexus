// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @boundary portable-stream-test-fixtures

use super::super::rows::PapercutTransfer;
use super::*;

fn papercut(id: &str, consolidated_into: Option<&str>) -> PapercutTransfer {
    PapercutTransfer {
        id: id.to_owned(),
        project_id: "project-a".to_owned(),
        title: format!("issue {id}"),
        description: String::new(),
        fingerprint: "a".repeat(64),
        reporter_token_hash: "b".repeat(64),
        status: if consolidated_into.is_some() {
            "consolidated".to_owned()
        } else {
            "open".to_owned()
        },
        consolidated_into: consolidated_into.map(str::to_owned),
        resolution: None,
        creator_agent: "agent".to_owned(),
        creator_session_id: "session".to_owned(),
        creator_model_id: None,
        creator_config_hash: "config".to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        resolved_at: None,
    }
}

#[test]
fn activity_facts_inventory_and_classification_history_round_trip() {
    use crate::analytics::types::*;
    let mut store = Store::open_memory().unwrap();
    let input:crate::coordination::domain::ToolHookInput=serde_json::from_value(serde_json::json!({"session_id":"s","agent":"pi","tool_use_id":"t","tool_name":"custom","tool_input":{}})).unwrap();
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
    store
        .record_inventory(
            "p",
            &input.context,
            "host_tools",
            &[Resource {
                name: "unused".into(),
                kind: ResourceKind::Tool,
                origin: Origin::HostTool,
                exposure: Exposure::Exposed,
                description: "Inspect checks".into(),
                activities: vec![],
                version: None,
            }],
            true,
        )
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
    assert!(store.reserve_activity_classification(&job, 10).unwrap());
    store
        .apply_activity_classification(
            &job,
            &CategoryDraft {
                category_id: "ci.inspect".into(),
                name: "Inspect CI".into(),
                definition: "Inspect continuous integration checks".into(),
                description: "Inspect check results".into(),
            },
            "fake:test",
        )
        .unwrap();
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let manifest = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut target = Store::open_memory().unwrap();
    target.import_streamed_snapshot(snapshot.path()).unwrap();
    assert_eq!(
        target
            .activity_summary(Some("p"))
            .unwrap()
            .classified_actions,
        1
    );
    assert_eq!(
        target
            .activity_catalog(Some("p"), 0, 64)
            .unwrap()
            .capabilities
            .len(),
        2
    );
    let mut bytes = Vec::new();
    assert_eq!(
        target.write_streamed_snapshot(&mut bytes).unwrap(),
        manifest
    );
}

#[test]
fn version_two_streams_remain_importable_and_retain_their_source_version() {
    let mut source = Store::open_memory().unwrap();
    let mut bytes = Vec::new();
    source.write_streamed_snapshot(&mut bytes).unwrap();
    let mut records: Vec<serde_json::Value> = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    records[0]["format_version"] = serde_json::json!(2);
    records.retain(|record| {
        !record["data"]["table"]
            .as_str()
            .is_some_and(|name| name.starts_with("analytics_"))
    });
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    for record in records {
        serde_json::to_writer(snapshot.as_file(), &record).unwrap();
        writeln!(snapshot.as_file()).unwrap();
    }
    let mut target = Store::open_memory().unwrap();
    assert!(matches!(
        target.import_streamed_snapshot(snapshot.path()).unwrap(),
        ImportOutcome::Imported { rows: 0, .. }
    ));
    let version = database_imports::table
        .select(database_imports::format_version)
        .first::<i32>(&mut target.connection)
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn consolidated_papercuts_are_inserted_after_their_canonical_dependencies() {
    let source = papercut("000-source", Some("zzz-canonical"));
    let canonical = papercut("zzz-canonical", None);

    let mut store = Store::open_memory().unwrap();
    for row in [&canonical, &source] {
        diesel::insert_into(papercuts::table)
            .values(row)
            .execute(&mut store.connection)
            .unwrap();
    }
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let manifest = store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut target = Store::open_memory().unwrap();
    assert!(matches!(
        target.import_streamed_snapshot(snapshot.path()).unwrap(),
        ImportOutcome::Imported { rows: 2, .. }
    ));
    assert_eq!(manifest.rows, 2);
    let registry = target.papercut_registry(Some("project-a")).unwrap();
    assert_eq!(registry.papercuts.len(), 2);
    let copied_source = registry
        .papercuts
        .iter()
        .find(|row| row.id == "000-source")
        .unwrap();
    assert_eq!(
        copied_source.consolidated_into.as_deref(),
        Some("zzz-canonical")
    );
}
