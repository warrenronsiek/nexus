// @feature persistence
// @feature agent-memory
// @spec docs/features/persistence.md
// @spec docs/features/agent-memory.md
use super::*;
use crate::memory::{MemoryProvenance, MemoryScope};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn concurrent_memory_appends_preserve_ordinals_across_connections() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").unwrap();
    let mut first = Store::connect(&url).unwrap();
    let mut second = Store::connect(&url).unwrap();
    let scope = MemoryScope::project(&format!("concurrent-memory-{}", uuid::Uuid::new_v4()));
    let provenance = MemoryProvenance {
        agent: "test".into(),
        session_id: "concurrent-append".into(),
        model_id: None,
        config_hash: "concurrency-test".into(),
    };
    let seed = first.add_memory(&scope, "seed", &provenance).unwrap();
    let (read_receiver, resume_sender) = pause_after_select(&mut first, "memory_spaces");
    let first_scope = scope.clone();
    let first_provenance = provenance.clone();
    let first_write =
        std::thread::spawn(move || first.add_memory(&first_scope, "first", &first_provenance));
    read_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_sender, done_receiver) = mpsc::channel();
    let second_write = std::thread::spawn(move || {
        let result = second.add_memory_from_lifecycle(&scope, "second", &provenance);
        done_sender.send(()).unwrap();
        result
    });
    let _ = done_receiver.recv_timeout(Duration::from_millis(200));
    resume_sender.send(()).unwrap();
    let first_result = first_write
        .join()
        .unwrap()
        .expect("the first append must commit");
    let second_result = second_write
        .join()
        .unwrap()
        .expect("the concurrent append must commit");
    let mut ordinals = [first_result.entry.ordinal, second_result.entry.ordinal];
    ordinals.sort();
    assert_eq!(ordinals, [seed.entry.ordinal + 1, seed.entry.ordinal + 2]);
}

fn pause_after_select(store: &mut Store, table: &str) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (read_sender, read_receiver) = mpsc::channel();
    let (resume_sender, resume_receiver) = mpsc::channel();
    let mut read_sender = Some(read_sender);
    let table = format!("\"{table}\"");
    store.connection.set_instrumentation(
        move |event: diesel::connection::InstrumentationEvent<'_>| {
            if let diesel::connection::InstrumentationEvent::FinishQuery { query, .. } = event {
                let query = query.to_string();
                if query.starts_with("SELECT") && query.contains(&table) {
                    if let Some(sender) = read_sender.take() {
                        sender.send(()).unwrap();
                        resume_receiver
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                }
            }
        },
    );
    (read_receiver, resume_sender)
}

#[test]
fn concurrent_conflict_resolution_and_escalation_preserve_history() {
    use crate::coordination::domain::{ConflictStatus, Severity};
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").unwrap();
    let mut first = Store::connect(&url).unwrap();
    let mut second = Store::connect(&url).unwrap();
    let ConflictFixture {
        project,
        left,
        right,
        conflict_id,
    } = seed_conflict(&mut first);
    let (read_receiver, resume_sender) = pause_after_select(&mut first, "conflicts");
    let first_project = project.clone();
    let escalation = std::thread::spawn(move || {
        let result = first.record_conflict(
            &first_project,
            &left,
            &right,
            overlap_advisory(Severity::Critical),
            "escalated",
        );
        (first, result)
    });
    read_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let second_conflict = conflict_id.clone();
    let (done_sender, done_receiver) = mpsc::channel();
    let resolution = std::thread::spawn(move || {
        let result = second.resolve_conflict(&second_conflict, "scope split");
        done_sender.send(()).unwrap();
        result
    });
    let _ = done_receiver.recv_timeout(Duration::from_millis(200));
    resume_sender.send(()).unwrap();
    let (mut first, result) = escalation.join().unwrap();
    result.unwrap();
    assert!(resolution.join().unwrap().unwrap());
    let projection = first.conflict_by_id(&conflict_id).unwrap().unwrap();
    assert_eq!(projection.severity, Severity::Critical);
    let events = first.list_events(Some(&project), 32).unwrap();
    let mut historical_status = ConflictStatus::Open;
    for event in events.iter().rev() {
        match event.kind.as_str() {
            "conflict_resolved" => historical_status = ConflictStatus::Resolved,
            "conflict_reopened" => historical_status = ConflictStatus::Open,
            _ => {}
        }
    }
    assert_eq!(
        projection.status, historical_status,
        "projection must agree with resolution/reopen history"
    );
}

struct ConflictFixture {
    project: String,
    left: crate::coordination::domain::Claim,
    right: crate::coordination::domain::Claim,
    conflict_id: String,
}

fn seed_conflict(store: &mut Store) -> ConflictFixture {
    use crate::coordination::domain::{Operation, PathIntent, Severity};
    let project = format!("concurrent-conflict-{}", uuid::Uuid::new_v4());
    let intent = PathIntent {
        path: "shared.rs".into(),
        operation: Operation::Write,
        line_start: None,
        line_end: None,
    };
    for session in ["left", "right"] {
        store
            .touch_session(&project, session, "test", None, "config")
            .unwrap();
    }
    let left = store
        .insert_claim(&project, "left", "original", &intent, 120)
        .unwrap();
    let right = store
        .insert_claim(&project, "right", "original", &intent, 120)
        .unwrap();
    let conflict_id = store
        .record_conflict(
            &project,
            &left,
            &right,
            overlap_advisory(Severity::Warning),
            "original",
        )
        .unwrap()
        .conflict_id
        .unwrap();
    ConflictFixture {
        project,
        left,
        right,
        conflict_id,
    }
}

fn overlap_advisory(
    severity: crate::coordination::domain::Severity,
) -> crate::coordination::domain::Advisory {
    crate::coordination::domain::Advisory {
        id: uuid::Uuid::new_v4().to_string(),
        severity,
        kind: "path_overlap".into(),
        message: "overlap".into(),
        path: "shared.rs".into(),
        other_session_id: "right".into(),
        conflict_id: None,
    }
}
