// @feature persistence
// @spec docs/features/persistence.md
use super::*;

#[test]
fn flyway_migrations_create_a_queryable_store() {
    let mut store = Store::open_memory().unwrap();
    assert_eq!(store.counts(120).unwrap().events, 0);
    assert!(store
        .list_claims(None, RecordScope::All)
        .unwrap()
        .is_empty());
}

#[test]
fn opening_store_waits_for_a_temporary_sqlite_write_lock() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("locked.db");
    drop(Store::open(&path).unwrap());

    let mut blocker = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let opener = blocker
        .exclusive_transaction::<_, anyhow::Error, _>(|_| {
            let (started, receiver) = std::sync::mpsc::channel();
            let open_path = path.clone();
            let opener = std::thread::spawn(move || {
                started.send(()).unwrap();
                Store::open(&open_path)
            });
            receiver.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(250));
            Ok(opener)
        })
        .unwrap();

    let mut reopened = opener.join().unwrap().unwrap();
    assert_eq!(reopened.counts(120).unwrap().events, 0);
}

#[test]
fn application_connection_waits_for_a_temporary_sqlite_write_lock() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("application-locked.db");
    let mut reader = Store::open(&path).unwrap();
    let mut blocker = DbConnection::establish(path.to_str().unwrap()).unwrap();

    let (query_started, observed_query) = std::sync::mpsc::channel();
    reader.connection.set_instrumentation(
        move |event: diesel::connection::InstrumentationEvent<'_>| {
            if matches!(
                event,
                diesel::connection::InstrumentationEvent::StartQuery { .. }
            ) {
                let _ = query_started.send(());
            }
        },
    );
    let (result_sender, result_receiver) = std::sync::mpsc::channel();

    let read = blocker
        .exclusive_transaction::<_, anyhow::Error, _>(|_| {
            let read = std::thread::spawn(move || {
                result_sender.send(reader.counts(120)).unwrap();
            });
            observed_query.recv_timeout(std::time::Duration::from_secs(5))?;
            assert!(matches!(
                result_receiver.recv_timeout(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            Ok(read)
        })
        .unwrap();

    assert_eq!(
        result_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .events,
        0
    );
    read.join().unwrap();
}

#[test]
fn oversized_event_payloads_are_replaced_with_a_bounded_marker() {
    let mut store = Store::open_memory().unwrap();
    append_event(
        &mut store.connection,
        "project",
        Some("session"),
        "oversized",
        json!({"output":"x".repeat(MAX_EVENT_PAYLOAD_BYTES + 1)}),
    )
    .unwrap();

    let events = store.list_events(Some("project"), 1).unwrap();
    let payload = serde_json::to_value(&events[0].payload).unwrap();
    assert_eq!(payload["truncated"], true);
    assert_eq!(payload["reason"], "event_payload_too_large");
    assert!(serde_json::to_vec(&payload).unwrap().len() <= MAX_EVENT_PAYLOAD_BYTES);
}

#[test]
fn large_valid_event_payloads_are_preserved_but_projected_from_ordinary_reads() {
    let mut store = Store::open_memory().unwrap();
    let content = "x".repeat(MAX_VISIBLE_EVENT_PAYLOAD_BYTES + 1);
    append_event(
        &mut store.connection,
        "project",
        Some("session"),
        "large_valid",
        json!({"content":content}),
    )
    .unwrap();

    let stored = events::table
        .select(events::payload_json)
        .first::<String>(&mut store.connection)
        .unwrap();
    assert!(stored.contains(&content));

    let visible = store.list_events(Some("project"), 1).unwrap();
    let payload = serde_json::to_value(&visible[0].payload).unwrap();
    assert_eq!(payload["reason"], "event_payload_omitted");
    assert_eq!(payload["original_bytes"], stored.len());
}

#[test]
fn event_reads_and_dashboard_windows_have_server_owned_hard_caps() {
    let mut store = Store::open_memory().unwrap();
    for index in 0..(crate::coordination::api::MAX_EVENT_RESULTS + 2) {
        append_event(
            &mut store.connection,
            "project",
            Some("session"),
            "bounded",
            json!({"index":index}),
        )
        .unwrap();
    }

    assert_eq!(
        store
            .list_events(Some("project"), usize::MAX)
            .unwrap()
            .len(),
        crate::coordination::api::MAX_EVENT_RESULTS
    );
    let dashboard = store
        .dashboard(Some("project"), usize::MAX, 1, 120)
        .unwrap();
    assert_eq!(
        dashboard.events.items.len(),
        crate::coordination::api::MAX_DASHBOARD_EVENT_RESULTS
    );
    assert!(dashboard.events.truncated);
}

#[test]
fn reconciliation_selects_only_the_most_recent_bounded_sessions() {
    let mut store = Store::open_memory().unwrap();
    let base = Utc::now();
    for index in 0..66 {
        let session_id = format!("session-{index:02}");
        store
            .touch_session(
                "project",
                &session_id,
                "codex",
                Some("/tmp/nexus-project"),
                "config",
            )
            .unwrap();
        diesel::update(
            sessions::table
                .filter(sessions::project_id.eq("project"))
                .filter(sessions::session_id.eq(&session_id)),
        )
        .set(sessions::last_seen_at.eq((base - chrono::Duration::milliseconds(index)).to_rfc3339()))
        .execute(&mut store.connection)
        .unwrap();
    }

    let sessions = store.reconcilable_sessions(3_600).unwrap();

    assert_eq!(sessions.len(), 64);
    assert_eq!(sessions.first().unwrap().session_id, "session-00");
    assert_eq!(sessions.last().unwrap().session_id, "session-63");
}

#[test]
fn live_coordination_views_expire_without_erasing_history() {
    use crate::coordination::domain::ConflictScope;
    let mut store = Store::open_memory().unwrap();
    for session in ["left", "right", "abandoned"] {
        store
            .touch_session("project", session, "test", None, "config")
            .unwrap();
    }
    seed_overlap(&mut store);
    let old = (Utc::now() - Duration::days(30)).to_rfc3339();
    diesel::update(sessions::table)
        .set(sessions::last_seen_at.eq(&old))
        .execute(&mut store.connection)
        .unwrap();
    // Live claims retain their owners, but an abandoned observation is history.
    assert_eq!(store.counts(120).unwrap().active_sessions, 2);
    assert_eq!(
        store.list_sessions(RecordScope::Active, 120).unwrap().len(),
        2
    );
    assert_eq!(
        store
            .dashboard(Some("project"), 1, 16, 120)
            .unwrap()
            .sessions
            .items
            .len(),
        2
    );
    assert_eq!(store.counts(120).unwrap().open_conflicts, 1);
    store.stop_session("project", "left").unwrap();
    assert_eq!(store.counts(120).unwrap().open_conflicts, 0);
    assert!(store
        .list_conflicts(None, ConflictScope::Open)
        .unwrap()
        .is_empty());
    let dashboard = store.dashboard(Some("project"), 1, 16, 120).unwrap();
    assert_eq!(dashboard.counts.open_conflicts, 0);
    assert!(dashboard.conflicts.items.is_empty());
    assert_eq!(
        store
            .list_conflicts(None, ConflictScope::All)
            .unwrap()
            .len(),
        1
    );
    expire_claims(&mut store);
    assert_eq!(store.counts(120).unwrap().active_sessions, 0);
    assert_eq!(store.list_sessions(RecordScope::All, 120).unwrap().len(), 3);
}

#[test]
fn live_conflicts_follow_replacement_leases_and_project_identity() {
    use crate::coordination::domain::ConflictScope;
    let mut store = Store::open_memory().unwrap();
    let intent = write_intent("shared.rs");
    for project in ["project", "other"] {
        for session in ["left", "right"] {
            store
                .touch_session(project, session, "test", None, "config")
                .unwrap();
        }
    }
    seed_overlap(&mut store);
    expire_claims(&mut store);
    // Neither identically named owners in another project nor another path renew this overlap.
    for session in ["left", "right"] {
        store
            .insert_claim("other", session, "renewal", &intent, 120)
            .unwrap();
    }
    assert_eq!(store.counts(120).unwrap().open_conflicts, 0);
    assert!(store
        .list_conflicts(None, ConflictScope::Open)
        .unwrap()
        .is_empty());
    store
        .insert_claim("project", "left", "replacement", &intent, 120)
        .unwrap();
    let different_path = write_intent("different.rs");
    store
        .insert_claim(
            "project",
            "right",
            "replacement-other-path",
            &different_path,
            120,
        )
        .unwrap();
    assert_eq!(store.counts(120).unwrap().open_conflicts, 0);
    store
        .insert_claim("project", "right", "replacement", &intent, 120)
        .unwrap();
    assert_eq!(store.counts(120).unwrap().open_conflicts, 1);
    assert_eq!(
        store
            .list_conflicts(None, ConflictScope::Open)
            .unwrap()
            .len(),
        1
    );
    store
        .release_claims(
            "project",
            "right",
            &crate::coordination::domain::ClaimRelease::Path(intent.path),
        )
        .unwrap();
    assert_eq!(store.counts(120).unwrap().open_conflicts, 0);
    assert_eq!(
        store
            .list_conflicts(None, ConflictScope::All)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn recent_session_reads_use_the_configured_activity_window() {
    let mut store = Store::open_memory().unwrap();
    store
        .touch_session("project", "waiting", "test", None, "config")
        .unwrap();
    let last_seen = (Utc::now() - Duration::seconds(300)).to_rfc3339();
    diesel::update(sessions::table)
        .set(sessions::last_seen_at.eq(last_seen))
        .execute(&mut store.connection)
        .unwrap();
    assert_eq!(store.counts(120).unwrap().active_sessions, 0);
    assert_eq!(store.counts(600).unwrap().active_sessions, 1);
    assert_eq!(
        store.list_sessions(RecordScope::Active, 600).unwrap().len(),
        1
    );
    assert_eq!(
        store
            .dashboard(Some("project"), 1, 1, 600)
            .unwrap()
            .counts
            .active_sessions,
        1
    );
}

fn seed_overlap(store: &mut Store) {
    use crate::coordination::domain::{Advisory, Severity};
    let intent = write_intent("shared.rs");
    let left = store
        .insert_claim("project", "left", "original", &intent, 120)
        .unwrap();
    let right = store
        .insert_claim("project", "right", "original", &intent, 120)
        .unwrap();
    store
        .record_conflict(
            "project",
            &left,
            &right,
            Advisory {
                id: Uuid::new_v4().to_string(),
                severity: Severity::Warning,
                kind: "path_overlap".into(),
                message: "overlap".into(),
                path: intent.path,
                other_session_id: "right".into(),
                conflict_id: None,
            },
            "original",
        )
        .unwrap();
}

fn expire_claims(store: &mut Store) {
    let expired = (Utc::now() - Duration::days(1)).to_rfc3339();
    diesel::update(claims::table)
        .set(claims::expires_at.eq(expired))
        .execute(&mut store.connection)
        .unwrap();
}

fn write_intent(path: &str) -> PathIntent {
    PathIntent {
        path: path.into(),
        operation: crate::coordination::domain::Operation::Write,
        line_start: None,
        line_end: None,
    }
}
