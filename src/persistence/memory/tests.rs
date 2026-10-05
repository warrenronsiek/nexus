// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

fn provenance(session_id: &str) -> MemoryProvenance {
    MemoryProvenance {
        agent: "codex".to_owned(),
        session_id: session_id.to_owned(),
        model_id: None,
        config_hash: "config".to_owned(),
    }
}

#[test]
fn application_writer_retries_a_short_sqlite_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("locked-memory.db");
    let mut writer = Store::open(&path).unwrap();
    let mut blocker = SqliteConnection::establish(path.to_str().unwrap()).unwrap();

    let handle = blocker
        .exclusive_transaction::<_, anyhow::Error, _>(|_| {
            let writer = std::thread::spawn(move || {
                writer.add_memory(
                    &MemoryScope::project("project-a"),
                    "wait for the lock",
                    &provenance("session-a"),
                )
            });
            std::thread::sleep(std::time::Duration::from_millis(250));
            Ok(writer)
        })
        .unwrap();

    assert!(handle.join().unwrap().unwrap().inserted);
}

#[test]
fn sqlite_rejects_raw_memory_update_and_delete() {
    let mut store = Store::open_memory().unwrap();
    let written = store
        .add_memory(
            &MemoryScope::Global,
            "raw memories stay immutable",
            &provenance("session-a"),
        )
        .unwrap();

    assert!(
        diesel::update(memory_entries::table.find(&written.entry.id))
            .set(memory_entries::content.eq("changed"))
            .execute(&mut store.connection)
            .is_err()
    );
    assert!(
        diesel::delete(memory_entries::table.find(&written.entry.id))
            .execute(&mut store.connection)
            .is_err()
    );
    assert_eq!(
        store
            .search_memories(&MemoryReadScope::Global, "immutable", 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn derived_summary_changes_bump_stream_revision_without_touching_raw_entries() {
    let mut store = Store::open_memory().unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance("session-a"))
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance("session-a"))
        .unwrap();
    assert_eq!(
        find_space(&mut store.connection, &scope)
            .unwrap()
            .unwrap()
            .revision,
        2
    );

    let job = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    let MemorySummaryInsert::Inserted(summary) = store
        .insert_memory_summary(&job, "the pair", "codex", Some("test-model"))
        .unwrap()
    else {
        panic!("expected a new summary")
    };
    assert_eq!(
        find_space(&mut store.connection, &scope)
            .unwrap()
            .unwrap()
            .revision,
        3
    );

    assert_eq!(store.invalidate_memory_summary(&summary.id).unwrap(), 1);
    assert_eq!(
        find_space(&mut store.connection, &scope)
            .unwrap()
            .unwrap()
            .revision,
        4
    );
    assert_eq!(
        load_entries(&mut store.connection, &scope).unwrap().len(),
        2
    );
}

#[test]
fn ready_compactions_propagate_corrupt_cooldown_metadata() {
    let mut store = Store::open_memory().unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance("session-a"))
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance("session-a"))
        .unwrap();
    let now = Utc::now();
    let job = store
        .ready_memory_compactions(&["codex"], now, 8)
        .unwrap()
        .remove(0);
    store
        .record_memory_compaction_attempt(
            &job,
            &MemoryCompactionAttempt {
                provider: "codex".to_owned(),
                model_id: None,
                outcome: MemoryCompactionOutcome::Failed,
                diagnostic: None,
                was_fallback: false,
                attempted_at: now,
            },
        )
        .unwrap();
    diesel::update(memory_compaction_attempts::table)
        .set(memory_compaction_attempts::next_retry_at.eq(Some("not-a-timestamp")))
        .execute(&mut store.connection)
        .unwrap();

    assert!(store
        .memory_provider_can_attempt(&job, "codex", now)
        .is_err());
}

#[test]
fn stale_frontier_does_not_consume_the_once_per_session_activation() {
    let mut store = Store::open_memory().unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(
            &scope,
            "remember the bounded frontier",
            &provenance("session-a"),
        )
        .unwrap();
    let space = find_space(&mut store.connection, &scope).unwrap().unwrap();
    diesel::update(memory_spaces::table.find(&space.id))
        .set(memory_spaces::revision.eq(memory_spaces::revision + 1))
        .execute(&mut store.connection)
        .unwrap();

    assert!(store
        .activate_memory_context(
            &MemoryReadScope::project("project-a"),
            "codex",
            "first-prompt",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_none());
    assert_eq!(
        memory_activations::table
            .count()
            .get_result::<i64>(&mut store.connection)
            .unwrap(),
        0
    );

    let repaired = store
        .memory_context(
            &MemoryReadScope::project("project-a"),
            MemoryContextLimits::default(),
        )
        .unwrap();
    assert_eq!(repaired.nodes.len(), 1);
    assert!(store
        .activate_memory_context(
            &MemoryReadScope::project("project-a"),
            "codex",
            "retry-after-repair",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_some());
}
