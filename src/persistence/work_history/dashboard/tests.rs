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
    let response = dashboard(
        &mut store.connection,
        &project,
        None,
        WorkSessionOrigin::All,
    )
    .unwrap();
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

#[test]
fn host_sessions_remain_selectable_behind_newer_commands_and_activity_floods() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    record(
        &mut store,
        &project,
        vec![
            observation(
                "host-open",
                "session",
                "host-root",
                json!({"event":"session_opened","title":"Named harness session","context":{"host_session_id":"native-thread","labels":{"nexus.capture.adapter":"host-hooks-v1","nexus.capture.host":"fixture-host"}}}),
            ),
            observation(
                "tool-open",
                "activity",
                "tool",
                json!({"event":"activity_declared","name":"Test invocation","kind":"test.run","work_session_id":"host-root","context":{}}),
            ),
        ],
    );
    populate_unrelated_dashboard_history(&mut store, &project);
    let action: WorkAction =
        serde_json::from_value(json!({"action":"dashboard","session_origin":"host"})).unwrap();
    let actor = WorkActor {
        agent: "fixture".into(),
        session_id: "reader".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let WorkResponse::Dashboard { dashboard: view } =
        store.work_command(&project, &actor, &action).unwrap()
    else {
        panic!("dashboard")
    };
    assert_eq!(view.sessions.len(), 1);
    assert_eq!(view.sessions[0].subject.id(), "host-root");
    assert!(
        view.truncated,
        "Activity source coverage must remain disclosed"
    );
    let WorkResponse::Dashboard {
        dashboard: selected,
    } = dashboard(
        &mut store.connection,
        &project,
        Some("host-root"),
        WorkSessionOrigin::All,
    )
    .unwrap()
    else {
        panic!("dashboard")
    };
    assert_eq!(selected.activities.len(), 1);
    assert_eq!(selected.activities[0].title, "Test invocation");
}

fn populate_unrelated_dashboard_history(store: &mut Store, project: &str) {
    for index in 0..4300 {
        record(
            store,
            project,
            vec![observation(
                &format!("caller-{index}-open"),
                "session",
                &format!("caller-{index}"),
                json!({"event":"session_opened","title":"Standalone command","context":{}}),
            )],
        );
    }
    // Only the inventory population matters for the flood: missing observations
    // remain honest projection gaps, rather than invented command measurements.
    for batch in 0..70 {
        let rows: Vec<_> = (0..64)
            .map(|index| SubjectRow {
                project_id: project.to_owned(),
                scope: "activity".into(),
                id: format!("flood-{}", batch * 64 + index),
                title: "Operation".into(),
                kind: "tool".into(),
                repository: None,
                host_session_id: None,
                environment: None,
                outcome: None,
                has_host_capture: false,
                occurred_at: "2026-10-30T00:00:00Z".into(),
                recorded_at: "2099-01-01T00:00:00Z".into(),
            })
            .collect();
        diesel::insert_into(work_subjects::table)
            .values(&rows)
            .execute(&mut store.connection)
            .unwrap();
    }
}

#[test]
fn harness_dashboard_includes_named_legacy_roots_and_excludes_standalone_cli_capture() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let legacy = "58448194a44a55a3e6f7e0e780fdcfe223da3649b0c53a4ca1e0b85eadd89329";
    let mut opened = observation(
        "legacy-open",
        "session",
        legacy,
        json!({"event":"session_opened","title":"codex session legacy-native","context":{"host_session_id":"legacy-native"}}),
    );
    opened.producer.name = "nexus-companion".into();
    opened.producer.version = "0.1.0".into();
    let renamed = observation(
        "legacy-title",
        "session",
        legacy,
        json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":"Historical named harness"}}}),
    );
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "legacy-native".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    store
        .work_command(
            &project,
            &actor,
            &WorkAction::Record {
                observations: vec![opened, renamed],
            },
        )
        .unwrap();
    record(
        &mut store,
        &project,
        vec![observation(
            "command-open",
            "session",
            "command-root",
            json!({"event":"session_opened","title":"Standalone command","context":{"labels":{"nexus.capture.host":"nexus-cli"}}}),
        )],
    );
    let WorkResponse::Dashboard { dashboard: view } = dashboard(
        &mut store.connection,
        &project,
        None,
        WorkSessionOrigin::Host,
    )
    .unwrap() else {
        panic!("dashboard")
    };
    assert_eq!(view.sessions.len(), 1);
    assert_eq!(view.sessions[0].subject.id(), legacy);
    assert_eq!(view.sessions[0].title, "Historical named harness");
}
