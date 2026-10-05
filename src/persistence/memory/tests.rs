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

fn observe_failed_commit(connection: &mut SqliteConnection) -> std::sync::mpsc::Receiver<()> {
    let (commit_failed, observed_failure) = std::sync::mpsc::channel();
    connection.set_instrumentation(move |event: diesel::connection::InstrumentationEvent<'_>| {
        if let diesel::connection::InstrumentationEvent::FinishQuery {
            query,
            error: Some(_),
            ..
        } = event
        {
            if query.to_string() == "COMMIT" {
                let _ = commit_failed.send(());
            }
        }
    });
    observed_failure
}

#[test]
fn memory_writer_retries_when_a_reader_blocks_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("reader-locked-memory.db");
    let mut writer = Store::open(&path).unwrap();
    let mut reader = Store::open(&path).unwrap();
    let observed_failure = observe_failed_commit(&mut writer.connection);

    let handle = reader
        .connection
        .transaction::<_, anyhow::Error, _>(|connection| {
            memory_entries::table
                .count()
                .get_result::<i64>(connection)?;
            let handle = std::thread::spawn(move || {
                let result = writer.add_memory(
                    &MemoryScope::Global,
                    "retry the reader-blocked commit",
                    &provenance("session-a"),
                );
                (writer, result)
            });
            observed_failure.recv_timeout(std::time::Duration::from_secs(5))?;
            Ok(handle)
        })
        .unwrap();

    let (mut writer, result) = handle.join().unwrap();
    let written = result.unwrap();
    assert!(written.inserted);
    assert_eq!(written.entry.ordinal, 0);
    let next = writer
        .add_memory(
            &MemoryScope::Global,
            "the connection remains usable",
            &provenance("session-a"),
        )
        .unwrap();
    assert_eq!(next.entry.ordinal, 1);
    let persisted = reader
        .search_memories(&MemoryReadScope::Global, ".*", 10)
        .unwrap();
    let ids = persisted
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![next.entry.id, written.entry.id]);
}

#[test]
fn failed_session_commit_does_not_poison_memory_writes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("reader-locked-session.db");
    let mut writer = Store::open(&path).unwrap();
    let mut reader = Store::open(&path).unwrap();

    reader
        .connection
        .transaction::<_, anyhow::Error, _>(|connection| {
            memory_entries::table
                .count()
                .get_result::<i64>(connection)?;
            let error = writer
                .touch_session("project-a", "failed-session", "codex", None, "config")
                .unwrap_err();
            assert!(sqlite_is_locked(&error), "{error}");
            Ok(())
        })
        .unwrap();

    let written = writer
        .add_memory(
            &MemoryScope::Global,
            "a failed lifecycle write must not block memories",
            &provenance("session-a"),
        )
        .unwrap();
    assert!(written.inserted);
    assert_eq!(reader.counts().unwrap().active_sessions, 0);
    assert_eq!(reader.counts().unwrap().events, 0);
    let persisted = reader
        .search_memories(&MemoryReadScope::Global, ".*", 10)
        .unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, written.entry.id);
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
