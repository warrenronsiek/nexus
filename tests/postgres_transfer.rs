// @feature persistence
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary external-postgres-and-json
#![cfg(feature = "postgres")]

use nexus::memory::{MemoryReadScope, MemoryScope};
use nexus::papercuts::{PapercutActor, PapercutStatus};
use nexus::persistence::{ImportOutcome, PortableSnapshot, Store};
use serde_json::json;
use std::io::Cursor;

const PROJECT: &str = "sqlite-transfer-fixture";
const AT: &str = "2026-01-02T03:04:05Z";

#[test]
fn sqlite_stream_import_preserves_relationships_and_advances_the_event_identity() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL integration test");
    let snapshot = transfer_fixture();
    let directory = tempfile::tempdir().unwrap();
    let stream = directory.path().join("snapshot.jsonl");
    write_fixture_stream(&snapshot, &stream);
    let mut store = Store::connect(&database_url).unwrap();

    assert!(matches!(
        store.import_streamed_snapshot(&stream).unwrap(),
        ImportOutcome::Imported { rows: 8, .. }
    ));
    assert!(matches!(
        store.import_streamed_snapshot(&stream).unwrap(),
        ImportOutcome::AlreadyImported { rows: 8, .. }
    ));

    let memories = store
        .search_memories(
            &MemoryReadScope::project(PROJECT),
            "relationship survived",
            10,
        )
        .unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].scope, MemoryScope::project(PROJECT));

    let papercuts = store.papercut_registry(Some(PROJECT)).unwrap();
    assert_eq!(papercuts.papercuts.len(), 2);
    let source = papercuts
        .papercuts
        .iter()
        .find(|papercut| papercut.id == "000-source")
        .unwrap();
    assert_eq!(source.status, PapercutStatus::Consolidated);
    assert_eq!(source.consolidated_into.as_deref(), Some("zzz-canonical"));
    assert_eq!(papercuts.upvotes[0].papercut_id, "zzz-canonical");
    assert_eq!(papercuts.upvotes[0].reported_papercut_id, "000-source");

    let sos = store.sos_registry(Some(PROJECT)).unwrap();
    assert_eq!(sos.issues.len(), 1);
    assert_eq!(sos.issues[0].importance_count, 1);
    assert_eq!(sos.reports[0].issue_id.as_deref(), Some("sos-issue"));

    store
        .report_papercut(
            PROJECT,
            "Post-import event",
            "proves the PostgreSQL identity moved beyond imported event 41",
            &PapercutActor {
                agent: "migration-test".to_owned(),
                session_id: "post-import".to_owned(),
                model_id: None,
                config_hash: "config".to_owned(),
            },
        )
        .unwrap();
    let events = store.list_events(Some(PROJECT), 64).unwrap();
    assert_eq!(events[0].id, 42);
    assert_eq!(events.last().unwrap().id, 41);
}

fn write_fixture_stream(snapshot: &PortableSnapshot, stream: &std::path::Path) {
    let mut output = Vec::new();
    snapshot.write_to(&mut output).unwrap();
    let fixture: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let mut records = vec![json!({"kind":"header", "format_version":2})];
    let tables = [
        "sessions",
        "claims",
        "conflicts",
        "advisories",
        "capability_uses",
        "events",
        "memory_spaces",
        "memory_entries",
        "memory_summaries",
        "memory_frontier_snapshots",
        "memory_compaction_queue",
        "memory_activations",
        "memory_compaction_attempts",
        "papercuts",
        "papercut_upvotes",
        "papercut_duplicate_reviews",
        "papercut_review_attempts",
        "sos_issues",
        "sos_reports",
        "sos_triage_attempts",
        "sos_issue_lifecycle",
    ];
    for table in tables {
        let mut rows = fixture[table].as_array().unwrap().clone();
        if table == "papercuts" {
            rows.reverse();
        }
        records.push(json!({"kind":"batch", "data":{"table":table, "rows":rows}}));
    }
    records.push(json!({"kind":"complete", "rows":8}));
    let encoded: String = records.iter().map(|record| format!("{record}\n")).collect();
    std::fs::write(&stream, encoded).unwrap();
}

fn transfer_fixture() -> PortableSnapshot {
    let snapshot = json!({
        "format_version": 1,
        "advisories": [],
        "capability_uses": [],
        "claims": [],
        "conflicts": [],
        "events": [{
            "id": 41,
            "project_id": PROJECT,
            "session_id": "import-session",
            "kind": "sqlite_import_fixture",
            "payload_json": "{}",
            "created_at": AT
        }],
        "memory_activations": [],
        "memory_compaction_attempts": [],
        "memory_compaction_queue": [],
        "memory_entries": [{
            "id": "memory-entry",
            "space_id": "memory-space",
            "ordinal": 0,
            "content": "This relationship survived migration.",
            "content_hash": "memory-hash",
            "agent": "migration-test",
            "session_id": "import-session",
            "model_id": null,
            "config_hash": "config",
            "created_at": AT
        }],
        "memory_frontier_snapshots": [],
        "memory_spaces": [{
            "id": "memory-space",
            "scope": "project",
            "project_id": PROJECT,
            "next_ordinal": 1,
            "revision": 1,
            "created_at": AT,
            "updated_at": AT
        }],
        "memory_summaries": [],
        "papercut_duplicate_reviews": [],
        "papercut_review_attempts": [],
        "papercut_upvotes": [{
            "id": "upvote",
            "project_id": PROJECT,
            "papercut_id": "zzz-canonical",
            "reported_papercut_id": "000-source",
            "reporter_agent": "migration-test",
            "reporter_session_id": "import-session",
            "reporter_model_id": null,
            "reporter_config_hash": "config",
            "note": "linked vote",
            "created_at": AT
        }],
        "papercuts": [
            {
                "id": "000-source",
                "project_id": PROJECT,
                "title": "Source issue",
                "description": "Consolidated before migration",
                "fingerprint": "1".repeat(64),
                "reporter_token_hash": "2".repeat(64),
                "status": "consolidated",
                "consolidated_into": "zzz-canonical",
                "resolution": null,
                "creator_agent": "migration-test",
                "creator_session_id": "import-session",
                "creator_model_id": null,
                "creator_config_hash": "config",
                "created_at": AT,
                "updated_at": AT,
                "resolved_at": null
            },
            {
                "id": "zzz-canonical",
                "project_id": PROJECT,
                "title": "Canonical issue",
                "description": "Retained after migration",
                "fingerprint": "3".repeat(64),
                "reporter_token_hash": "4".repeat(64),
                "status": "open",
                "consolidated_into": null,
                "resolution": null,
                "creator_agent": "migration-test",
                "creator_session_id": "import-session",
                "creator_model_id": null,
                "creator_config_hash": "config",
                "created_at": AT,
                "updated_at": AT,
                "resolved_at": null
            }
        ],
        "sessions": [],
        "sos_issue_lifecycle": [],
        "sos_issues": [{
            "id": "sos-issue",
            "project_id": PROJECT,
            "title": "Imported SOS issue",
            "summary": "One linked source report",
            "status": "open",
            "resolution": null,
            "created_at": AT,
            "updated_at": AT,
            "closed_at": null
        }],
        "sos_reports": [{
            "id": "sos-report",
            "project_id": PROJECT,
            "issue_id": "sos-issue",
            "message": "An imported problem",
            "report_type": "runtime",
            "environment": "ci",
            "version": "1",
            "model": "test-model",
            "trace": "",
            "metadata_json": "{}",
            "reporter_agent": "migration-test",
            "reporter_session_id": "import-session",
            "reporter_model_id": null,
            "reporter_config_hash": "config",
            "created_at": AT
        }],
        "sos_triage_attempts": []
    });
    PortableSnapshot::read_from(Cursor::new(serde_json::to_vec(&snapshot).unwrap())).unwrap()
}
