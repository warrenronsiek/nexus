// @feature persistence
// @spec docs/features/persistence.md
use super::*;

#[test]
fn flyway_migrations_create_a_queryable_store() {
    let mut store = Store::open_memory().unwrap();
    assert_eq!(store.counts().unwrap().events, 0);
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

    let mut blocker = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
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
    assert_eq!(reopened.counts().unwrap().events, 0);
}

#[test]
fn application_connection_waits_for_a_temporary_sqlite_write_lock() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("application-locked.db");
    let mut reader = Store::open(&path).unwrap();
    let mut blocker = SqliteConnection::establish(path.to_str().unwrap()).unwrap();

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
                result_sender.send(reader.counts()).unwrap();
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
