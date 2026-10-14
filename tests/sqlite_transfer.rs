// @feature persistence
// @spec docs/features/persistence.md
#![cfg(feature = "sqlite")]

use nexus::memory::{MemoryProvenance, MemoryReadScope, MemoryScope};
use nexus::persistence::{backup_sqlite_database, snapshot_sqlite_database, ImportOutcome, Store};

fn provenance() -> MemoryProvenance {
    MemoryProvenance {
        agent: "migration-test".to_owned(),
        session_id: "session-a".to_owned(),
        model_id: Some("model-a".to_owned()),
        config_hash: "config-a".to_owned(),
    }
}

#[test]
fn logical_snapshot_import_is_atomic_idempotent_and_refuses_a_different_source() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.db");
    let mut source = Store::open(&source_path).unwrap();
    source
        .add_memory(
            &MemoryScope::project("project-a"),
            "preserve this migration marker",
            &provenance(),
        )
        .unwrap();

    let snapshot = source.portable_snapshot().unwrap();
    let digest = snapshot.digest().unwrap();
    let target_path = temporary.path().join("target.db");
    let mut target = Store::open(&target_path).unwrap();

    assert!(matches!(
        target.import_snapshot(&snapshot, &digest).unwrap(),
        ImportOutcome::Imported { .. }
    ));
    assert_eq!(
        target
            .search_memories(
                &MemoryReadScope::project("project-a"),
                "migration marker",
                10,
            )
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        target.import_snapshot(&snapshot, &digest).unwrap(),
        ImportOutcome::AlreadyImported { .. }
    ));

    let other_path = temporary.path().join("other.db");
    let mut other = Store::open(&other_path).unwrap();
    let other_snapshot = other.portable_snapshot().unwrap();
    let other_digest = other_snapshot.digest().unwrap();
    let error = target
        .import_snapshot(&other_snapshot, &other_digest)
        .unwrap_err();
    assert!(format!("{error:#}").contains("different SQLite snapshot"));
}

#[test]
fn logical_snapshot_refuses_to_merge_into_unmarked_existing_data() {
    let temporary = tempfile::tempdir().unwrap();
    let mut source = Store::open(&temporary.path().join("source.db")).unwrap();
    let snapshot = source.portable_snapshot().unwrap();
    let digest = snapshot.digest().unwrap();

    let mut target = Store::open(&temporary.path().join("target.db")).unwrap();
    target
        .add_memory(
            &MemoryScope::project("project-a"),
            "already here",
            &provenance(),
        )
        .unwrap();

    let error = target.import_snapshot(&snapshot, &digest).unwrap_err();
    assert!(format!("{error:#}").contains("target database is not empty"));
}

#[test]
fn sqlite_snapshot_uses_a_read_only_online_copy_of_the_source() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.db");
    let mut source = Store::open(&source_path).unwrap();
    source
        .add_memory(
            &MemoryScope::project("project-a"),
            "copied without modifying the source",
            &provenance(),
        )
        .unwrap();
    drop(source);
    let before = std::fs::read(&source_path).unwrap();

    let snapshot = snapshot_sqlite_database(&source_path).unwrap();

    assert_eq!(std::fs::read(&source_path).unwrap(), before);
    let mut target = Store::open(&temporary.path().join("target.db")).unwrap();
    let digest = snapshot.digest().unwrap();
    target.import_snapshot(&snapshot, &digest).unwrap();
    assert_eq!(
        target
            .search_memories(
                &MemoryReadScope::project("project-a"),
                "modifying the source",
                10,
            )
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn sqlite_online_backup_can_stage_a_container_readable_source_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.db");
    let mut source = Store::open(&source_path).unwrap();
    source
        .add_memory(
            &MemoryScope::project("project-a"),
            "staged for the Compose container",
            &provenance(),
        )
        .unwrap();
    drop(source);
    let before = std::fs::read(&source_path).unwrap();
    let staged_path = temporary.path().join("staged.db");

    backup_sqlite_database(&source_path, &staged_path).unwrap();

    assert_eq!(std::fs::read(&source_path).unwrap(), before);
    let mut staged = Store::open(&staged_path).unwrap();
    assert_eq!(
        staged
            .search_memories(
                &MemoryReadScope::project("project-a"),
                "Compose container",
                10,
            )
            .unwrap()
            .len(),
        1
    );
    drop(staged);

    let mut permissions = std::fs::metadata(&staged_path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o444);
    std::fs::set_permissions(&staged_path, permissions).unwrap();
    let mut source = Store::open(&source_path).unwrap();
    source
        .add_memory(
            &MemoryScope::project("project-a"),
            "new state replaces the prior staged copy",
            &provenance(),
        )
        .unwrap();
    drop(source);

    backup_sqlite_database(&source_path, &staged_path).unwrap();

    let mut staged = Store::open(&staged_path).unwrap();
    assert_eq!(
        staged
            .search_memories(
                &MemoryReadScope::project("project-a"),
                "new state replaces",
                10,
            )
            .unwrap()
            .len(),
        1
    );
}
