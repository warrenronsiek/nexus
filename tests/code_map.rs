// @feature code-map
// @spec docs/features/code-map.md
#![cfg(feature = "sqlite")]

use nexus::code_map::{MapMeasure, MapQuery, MapSnapshot};
use nexus::persistence::Store;

#[test]
fn producer_dimensions_roll_up_and_replay_in_project_scope() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("map.sqlite")).unwrap();
    let snapshot: MapSnapshot = serde_json::from_value(serde_json::json!({
        "id": "example-1", "producer": "ci", "measure": "branch_load",
        "title": "Branch load", "unit": "branches", "reducer": "max",
        "revision": "abcdef", "observed_at": "2026-11-10T00:00:00Z",
        "samples": [
            {"subject": "parser", "value": 11, "dimensions": {"team": "language", "file": "src/parser.rs"}},
            {"subject": "lexer", "value": 4, "dimensions": {"team": "language", "file": "src/lexer.rs"}},
            {"subject": "runner", "value": 0, "dimensions": {"file": "src/runner.rs"}}
        ]
    })).unwrap();
    assert!(store.publish_code_map("alpha", "ci", &snapshot).unwrap());
    assert!(!store.publish_code_map("alpha", "ci", &snapshot).unwrap());
    let mut conflict = snapshot.clone();
    conflict.samples[0].value = 42.0;
    assert!(store.publish_code_map("alpha", "ci", &conflict).is_err());

    let query = MapQuery {
        measure: MapMeasure::Published {
            producer: "ci".into(),
            measure: "branch_load".into(),
        },
        group_by: Some("team".into()),
        ..MapQuery::default()
    };
    let map = store.code_map("alpha", &query).unwrap();
    assert_eq!(map.groups.len(), 2);
    assert_eq!(map.groups[0].label, "language");
    assert_eq!(map.groups[0].value, 11.0);
    assert_eq!(map.groups[0].weight, 2.0);
    assert_eq!(map.groups[1].label, "Unattributed");
    assert_eq!(map.groups[1].value, 0.0);
    assert_eq!(map.groups[0].evidence.len(), 2);
    assert!(store.code_map("beta", &query).is_err());

    let stream = tempfile::NamedTempFile::new().unwrap();
    store.write_streamed_snapshot(stream.as_file()).unwrap();
    let mut restored = Store::open(&temporary.path().join("restored.sqlite")).unwrap();
    restored.import_streamed_snapshot(stream.path()).unwrap();
    let restored_map = restored.code_map("alpha", &query).unwrap();
    assert_eq!(restored_map.groups[0].value, 11.0);
    assert_eq!(restored_map.source.unwrap().reporter, "ci");
}

#[test]
fn concern_reports_keep_custom_dimensions_and_count_shared_concerns_once() {
    use nexus::code_concerns::{ConcernAction, ConcernActor};
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("map.sqlite")).unwrap();
    let actor = ConcernActor {
        agent: "test".into(),
        session_id: "map".into(),
        model_id: None,
        config_hash: "map".into(),
    };
    let action: ConcernAction = serde_json::from_value(serde_json::json!({
        "action": "record",
        "observations": [
            {"id": "first", "concern_id": "issue", "observed_at": "2026-11-10T00:00:00Z",
             "body": {"event": "reported", "title": "A repeated problem", "description": "Description",
                      "encounter": {"dimensions": {"team": "search"}}}},
            {"id": "second", "concern_id": "issue", "observed_at": "2026-11-10T01:00:00Z",
             "body": {"event": "encountered", "encounter": {"dimensions": {"team": "storage"}}}}
        ]
    })).unwrap();
    store.concern_command("alpha", &actor, &action).unwrap();

    let mut query = MapQuery {
        group_by: Some("team".into()),
        ..MapQuery::default()
    };
    let encounters = store.code_map("alpha", &query).unwrap();
    assert_eq!(
        encounters
            .groups
            .iter()
            .map(|group| group.label.as_str())
            .collect::<Vec<_>>(),
        ["search", "storage"]
    );
    let default_view = store.code_map("alpha", &MapQuery::default()).unwrap();
    assert_eq!(default_view.group_by, "subject");
    assert_eq!(default_view.groups[0].samples, 2);
    query.measure = MapMeasure::OpenConcerns;
    let open = store.code_map("alpha", &query).unwrap();
    assert_eq!(open.groups.len(), 1);
    assert_eq!(open.groups[0].label, "Shared");
    assert_eq!(open.groups[0].value, 1.0);
    assert_eq!(
        open.groups[0].evidence[0].concern_id.as_deref(),
        Some("issue")
    );
}

fn concern_area_fixture() -> (tempfile::TempDir, Store) {
    use nexus::code_concerns::{ConcernAction, ConcernActor};
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("map.sqlite")).unwrap();
    let actor = ConcernActor {
        agent: "test".into(),
        session_id: "map".into(),
        model_id: None,
        config_hash: "map".into(),
    };
    let action: ConcernAction =
        serde_json::from_str(include_str!("fixtures/code_map_concern_placements.json")).unwrap();
    store.concern_command("alpha", &actor, &action).unwrap();
    (temporary, store)
}

#[test]
fn concern_map_preserves_area_identity_placement_and_measure_provenance() {
    let (_temporary, mut store) = concern_area_fixture();
    let default_view = store.code_map("alpha", &MapQuery::default()).unwrap();
    assert_eq!(default_view.group_by, "file");

    let query = MapQuery {
        group_by: Some("file".into()),
        ..MapQuery::default()
    };
    let encounters = store.code_map("alpha", &query).unwrap();
    assert_eq!(encounters.title, "Reported concern encounters");
    assert_eq!(encounters.unit, "encounters");
    assert_eq!(encounters.groups.len(), 3);
    assert_eq!(encounters.groups[0].label, "src/shared.rs");
    assert_eq!(encounters.groups[0].value, 2.0);
    assert_eq!(encounters.groups[0].samples, 2);
    assert_eq!(encounters.groups[0].evidence.len(), 1);
    assert_eq!(
        encounters.groups[0].evidence[0].reference.as_deref(),
        Some("a-old")
    );
    let file_keys = encounters
        .groups
        .iter()
        .filter(|group| group.label == "src/shared.rs")
        .map(|group| &group.key)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(file_keys.len(), 2);
    assert!(encounters
        .groups
        .iter()
        .any(|group| group.label == "Unattributed" && group.value == 1.0));
    assert!(!encounters.partial);
}

#[test]
fn open_concern_drilldown_counts_once_and_preserves_latest_provenance() {
    let (_temporary, mut store) = concern_area_fixture();
    let query = MapQuery {
        group_by: Some("file".into()),
        ..MapQuery::default()
    };
    let encounters = store.code_map("alpha", &query).unwrap();
    let open_query = MapQuery {
        measure: MapMeasure::OpenConcerns,
        bucket: Some(encounters.groups[0].key.clone()),
        ..query
    };
    let open = store.code_map("alpha", &open_query).unwrap();
    assert_eq!(open.title, "Open concerns");
    assert_eq!(open.unit, "concerns");
    assert_eq!(open.bucket_label.as_deref(), Some("src/shared.rs"));
    assert_eq!(open.groups.len(), 1);
    assert_eq!(open.groups[0].label, "src/shared.rs");
    assert_eq!(open.groups[0].value, 1.0);
    assert_eq!(
        open.groups[0].evidence[0].reference.as_deref(),
        Some("z-new")
    );
    assert!(store
        .code_map("beta", &open_query)
        .unwrap()
        .groups
        .is_empty());
}

#[test]
fn map_bounds_encoded_evidence_for_large_concern_records() {
    use nexus::code_concerns::{ConcernAction, ConcernActor};
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("map.sqlite")).unwrap();
    let actor = ConcernActor {
        agent: "test".into(),
        session_id: "map".into(),
        model_id: None,
        config_hash: "map".into(),
    };
    let detail = "\u{0001}".repeat(490);
    for group in 0..32 {
        let observations = (0..6).map(|item| serde_json::json!({
            "id": format!("event-{group}-{item}-{detail}"),
            "concern_id": format!("issue-{group}-{item}-{detail}"),
            "observed_at": "2026-11-10T00:00:00Z",
            "body": {"event": "reported", "title": format!("Concern {group}-{item} {detail}"),
                     "description": "Long retained evidence",
                     "encounter": {"dimensions": {"component": format!("component-{group}")}}}
        })).collect::<Vec<_>>();
        let action: ConcernAction = serde_json::from_value(serde_json::json!({
            "action": "record", "observations": observations,
        }))
        .unwrap();
        store.concern_command("alpha", &actor, &action).unwrap();
    }
    let query = MapQuery {
        group_by: Some("component".into()),
        ..MapQuery::default()
    };
    let map = store.code_map("alpha", &query).unwrap();
    let bytes = serde_json::to_vec(&map).unwrap().len();
    assert!(bytes <= 768 * 1024, "map encoded to {bytes} bytes");
    assert!(map.partial);
    assert_eq!(map.groups.len(), 32);
}
