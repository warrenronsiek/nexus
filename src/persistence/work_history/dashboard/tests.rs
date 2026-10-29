// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: closed work observations seed a shared-parent read budget.
use super::*;
use crate::persistence::{test_support::capture_queries, Store};
use serde_json::{json, Value};

fn store() -> Store {
    #[cfg(feature = "sqlite")]
    let store = Store::open_memory().unwrap();
    #[cfg(feature = "postgres")]
    let store = {
        let mut store = Store::connect(&std::env::var("NEXUS_TEST_DATABASE_URL").unwrap()).unwrap();
        store.connection.begin_test_transaction().unwrap();
        store
    };
    store
}
fn observation(id: &str, scope: &str, subject: &str, body: Value) -> WorkObservationInput {
    WorkObservationInput {
        id: id.into(),
        subject: if scope == "session" {
            WorkSubject::Session { id: subject.into() }
        } else {
            WorkSubject::Activity { id: subject.into() }
        },
        occurred_at: "2026-10-29T12:00:00Z".parse().unwrap(),
        source_clock: None,
        producer: WorkProducer {
            name: "fixture".into(),
            version: "1".into(),
            instance_id: "clock".into(),
        },
        method: "caller".into(),
        body: serde_json::from_value(body).unwrap(),
        correction: None,
    }
}
fn record(store: &mut Store, project: &str, observations: Vec<WorkObservationInput>) {
    let actor = WorkActor {
        agent: "fixture".into(),
        session_id: "host".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let action = WorkAction::Record { observations };
    store.work_command(project, &actor, &action).unwrap();
}

#[test]
fn shared_parent_dashboard_keeps_a_bounded_query_budget_and_exact_membership() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    record(
        &mut store,
        &project,
        vec![
            observation(
                "root-open",
                "session",
                "root",
                json!({"event":"session_opened","title":"Root","context":{}}),
            ),
            observation(
                "parent-open",
                "activity",
                "parent",
                json!({"event":"activity_declared","name":"Parent","kind":"agent","work_session_id":"root","context":{}}),
            ),
        ],
    );
    for start in (0..64).step_by(16) {
        let mut events = vec![];
        for index in start..start + 16 {
            let id = format!("task-{index}");
            events.push(observation(&format!("{id}-open"), "activity", &id,
                json!({"event":"activity_declared","name":id,"kind":"test.unit","parent_activity_id":"parent","context":{}})));
            events.push(observation(&format!("{id}-finish"), "activity", &id,
                json!({"event":"finished","outcome":"succeeded","elapsed":{"nanoseconds":"1000000000","method":"monotonic","clock_id":"clock"}})));
        }
        record(&mut store, &project, events);
    }
    let queries = capture_queries(&mut store.connection);
    let response = dashboard(&mut store.connection, &project, None).unwrap();
    let WorkResponse::Dashboard { dashboard: result } = response else {
        panic!("expected dashboard")
    };
    assert_eq!(result.scanned_subjects, 66);
    assert_eq!(result.unlinked_activity_count, 0);
    let tests = result
        .activity_kinds
        .iter()
        .find(|item| item.kind == "test.unit")
        .unwrap();
    assert_eq!(tests.duration.known_count, 64);
    assert_eq!(tests.duration.total_seconds, 64.0);
    let count = queries.lock().unwrap().len();
    assert!(
        count <= 30,
        "64 shared-parent activities used {count} queries"
    );
    println!("64 shared-parent activities: {count} queries");
}

#[test]
fn partial_prefetch_preserves_projection_gaps_and_later_subjects() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    record(
        &mut store,
        &project,
        vec![
            observation(
                "000-open",
                "session",
                "a-heavy",
                json!({"event":"session_opened","title":"Heavy root","context":{}}),
            ),
            observation(
                "later-open",
                "session",
                "z-later",
                json!({"event":"session_opened","title":"Later root","context":{}}),
            ),
        ],
    );
    for batch in 0..16 {
        let events = (0..64)
            .map(|i| {
                observation(
                    &format!("milestone-{:04}", batch * 64 + i),
                    "session",
                    "a-heavy",
                    json!({"event":"milestone","name":"Progress"}),
                )
            })
            .collect();
        record(&mut store, &project, events);
    }
    let subjects = [
        WorkSubject::Session {
            id: "a-heavy".into(),
        },
        WorkSubject::Session {
            id: "z-later".into(),
        },
    ];
    let expected: Vec<_> = subjects
        .iter()
        .map(|subject| {
            projection::Projection::new(&mut store.connection, &project)
                .detail(subject)
                .unwrap()
                .unwrap()
        })
        .collect();
    assert!(expected[0]
        .summary
        .gaps
        .iter()
        .any(|gap| gap == "projection_limit"));
    let mut reader = projection::Projection::new(&mut store.connection, &project);
    reader.prefetch(&subjects).unwrap();
    for (subject, expected) in subjects.iter().zip(expected) {
        let actual = reader.detail(subject).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}
