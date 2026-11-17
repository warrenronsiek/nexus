// @feature observability-ui
// @feature persistence
// @spec docs/features/observability-ui.md
// @spec docs/features/persistence.md
// @boundary disposable-postgres: this test uses only NEXUS_TEST_DATABASE_URL.
use super::*;
use crate::persistence::models::{NewClaim, NewConflict};
use std::time::{Duration, Instant};

#[test]
fn dashboard_liveness_stays_responsive_with_large_session_claim_history() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").expect("disposable PostgreSQL URL required");
    let mut store = Store::connect(&url).unwrap();
    store.connection.begin_test_transaction().unwrap();
    let project = format!("dashboard-scale-{}", uuid::Uuid::new_v4());
    for session in ["left", "right"] {
        seed_owner_claims(&mut store, &project, session);
    }
    seed_conflicts(&mut store, &project);

    let started = Instant::now();
    let dashboard = store.dashboard(Some(&project), 32, 16, 120).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(dashboard.counts.open_conflicts, 1);
    assert_eq!(dashboard.conflicts.items.len(), 1);
    assert_eq!(dashboard.conflicts.items[0].path, "src/live.rs");
    assert_eq!(dashboard.counts.active_claims, 2);
    assert!(
        elapsed < Duration::from_secs(2),
        "dashboard exceeded its read budget: {elapsed:?}"
    );
    println!("Dashboard with 60,000 historical claims: {elapsed:?}");
}

fn fixture_paths(count: usize) -> impl Iterator<Item = (String, String)> {
    (0..count)
        .map(|index| (index.to_string(), format!("src/history-{index}.rs")))
        .chain(std::iter::once(("live".into(), "src/live.rs".into())))
}

fn seed_owner_claims(store: &mut Store, project: &str, session: &str) {
    store
        .touch_session(project, session, "test", None, "config")
        .unwrap();
    let now = Utc::now().to_rfc3339();
    let expired = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let live = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let identities: Vec<_> = fixture_paths(30_000)
        .map(|(tool, path)| (format!("{project}-{session}-{tool}"), tool, path))
        .collect();
    for chunk in identities.chunks(500) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|(id, tool, path)| NewClaim {
                id,
                project_id: project,
                session_id: session,
                tool_use_id: tool,
                path,
                operation: "write",
                line_start: -1,
                line_end: -1,
                state: if tool == "live" {
                    "modified"
                } else {
                    "released"
                },
                expires_at: if tool == "live" { &live } else { &expired },
                updated_at: &now,
            })
            .collect();
        diesel::insert_into(claims::table)
            .values(&rows)
            .execute(&mut store.connection)
            .unwrap();
    }
}

fn seed_conflicts(store: &mut Store, project: &str) {
    let now = Utc::now().to_rfc3339();
    for (suffix, path) in fixture_paths(256) {
        let row = NewConflict {
            id: &format!("{project}-{suffix}"),
            project_id: project,
            left_claim_id: &format!("{project}-left-{suffix}"),
            right_claim_id: &format!("{project}-right-{suffix}"),
            path: &path,
            severity: "warning",
            kind: "file_overlap",
            status: "open",
            message: "Fixture overlap",
            created_at: &now,
            updated_at: &now,
        };
        diesel::insert_into(conflicts::table)
            .values(row)
            .execute(&mut store.connection)
            .unwrap();
    }
}

#[test]
fn dominant_project_event_count_does_not_scan_payload_history() {
    use crate::persistence::test_support::{capture_queries, postgres_query_plan, QueryPlanMode};
    use diesel::connection::SimpleConnection;
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").expect("disposable PostgreSQL URL required");
    let mut store = Store::connect(&url).unwrap();
    let project = format!("count-{}", uuid::Uuid::new_v4());
    seed_event_count_history(&mut store, &project);
    // Committed append-only history has visible heap pages, as on the live store.
    store
        .connection
        .batch_execute("VACUUM (ANALYZE) events")
        .unwrap();
    let queries = capture_queries(&mut store.connection);
    let dashboard = store.dashboard(Some(&project), 1, 1, 120).unwrap();
    let query = queries
        .lock()
        .unwrap()
        .iter()
        .find(|sql| sql.starts_with("SELECT COUNT(*) FROM \"events\" WHERE"))
        .expect("capture the production event count")
        .clone();
    let plan = postgres_query_plan(&mut store.connection, &query, QueryPlanMode::Estimated);
    diesel::delete(events::table.filter(events::project_id.eq(&project)))
        .execute(&mut store.connection)
        .unwrap();
    assert_eq!(dashboard.counts.events, 20_000);
    assert!(
        !plan.contains("Seq Scan on events"),
        "event counts must avoid payload history scans:\n{plan}"
    );
}

fn seed_event_count_history(store: &mut Store, project: &str) {
    let now = Utc::now();
    let times: Vec<_> = (0..20_000)
        .map(|i| (now + chrono::Duration::microseconds(i)).to_rfc3339())
        .collect();
    for page in times.chunks(500) {
        let rows: Vec<_> = page
            .iter()
            .map(|time| {
                (
                    events::project_id.eq(project),
                    events::session_id.eq("fixture"),
                    events::kind.eq("fixture"),
                    events::payload_json.eq("{}"),
                    events::created_at.eq(time),
                )
            })
            .collect();
        diesel::insert_into(events::table)
            .values(rows)
            .execute(&mut store.connection)
            .unwrap();
    }
}
