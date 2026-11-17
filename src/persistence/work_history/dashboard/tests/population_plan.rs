// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary disposable-postgres-plan: controlled membership fixtures and captured SELECTs are measured only in NEXUS_TEST_DATABASE_URL.
use super::*;
use crate::persistence::schema::work_links;
use crate::persistence::test_support::{postgres_query_plan, QueryPlanMode};
use diesel::connection::SimpleConnection;

#[test]
fn whole_root_counts_do_not_fetch_membership_payloads() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").unwrap();
    let mut store = Store::connect(&url).unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    for start in (0..20_000).step_by(250) {
        let rows: Vec<_> = (start..start + 250)
            .map(|i| {
                let payload: String = (0..12).map(|_| uuid::Uuid::new_v4().to_string()).collect();
                (
                    work_links::project_id.eq(&project),
                    work_links::event_id.eq(format!("event-{i}")),
                    work_links::source_scope.eq("activity"),
                    work_links::source_id.eq(format!("source-{i:05}")),
                    work_links::relation.eq("member_of"),
                    work_links::target_scope.eq("session"),
                    work_links::target_id.eq("root"),
                    work_links::target_json.eq(payload),
                )
            })
            .collect();
        diesel::insert_into(work_links::table)
            .values(rows)
            .execute(&mut store.connection)
            .unwrap();
    }
    // Rolled-back fixture projects leave physical index pages behind. Compare
    // compact migrated indexes rather than a previous test's empty index bloat.
    store
        .connection
        .batch_execute("REINDEX INDEX work_links_target")
        .unwrap();
    store
        .connection
        .batch_execute("VACUUM (ANALYZE) work_links")
        .unwrap();
    let queries = capture_queries(&mut store.connection);
    let sample =
        population::member_sample(&mut store.connection, &project, &["root".into()]).unwrap();
    let captured = queries
        .lock()
        .unwrap()
        .iter()
        .find(|q| q.contains("count( DISTINCT"))
        .unwrap()
        .clone();
    let plan = postgres_query_plan(&mut store.connection, &captured, QueryPlanMode::Estimated);
    diesel::delete(work_links::table.filter(work_links::project_id.eq(&project)))
        .execute(&mut store.connection)
        .unwrap();
    assert!(sample.truncated);
    assert!(
        plan.contains("Index Only Scan"),
        "membership counts must avoid fetching payload rows:\n{plan}"
    );
}

#[test]
fn request_lookup_does_not_scan_the_activity_population() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    let roots: Vec<_> = (0..141).map(|i| format!("root-{i:03}")).collect();
    seed_request_history(&mut store, &project, &roots);
    store
        .connection
        .batch_execute("ANALYZE work_subjects; ANALYZE work_links")
        .unwrap();
    let queries = capture_queries(&mut store.connection);
    let (requests, truncated) =
        population::request_rows(&mut store.connection, &project, &roots).unwrap();
    assert_eq!(requests.len(), 100);
    assert!(!truncated);
    let captured = queries.lock().unwrap()[0].clone();
    let plan = postgres_query_plan(&mut store.connection, &captured, QueryPlanMode::Analyzed);
    assert!(
        !plan.contains("Seq Scan on work_subjects"),
        "request lookup must not scan unrelated activity kinds:\n{plan}"
    );
}

fn seed_request_history(store: &mut Store, project: &str, roots: &[String]) {
    use crate::persistence::schema::work_subjects;
    let subjects: Vec<_> = roots
        .iter()
        .cloned()
        .map(|id| ("session", id, "agent"))
        .chain((0..20_000).map(|i| {
            (
                "activity",
                format!("source-{i:05}"),
                if i % 200 == 0 { "request" } else { "tool" },
            )
        }))
        .map(|(scope, id, kind)| {
            (
                work_subjects::project_id.eq(project),
                work_subjects::scope.eq(scope),
                work_subjects::id.eq(id),
                work_subjects::title.eq("Fixture"),
                work_subjects::kind.eq(kind),
                work_subjects::occurred_at.eq("2026-01-01T00:00:00Z"),
                work_subjects::recorded_at.eq("2026-01-01T00:00:00Z"),
            )
        })
        .collect();
    for page in subjects.chunks(500) {
        diesel::insert_into(work_subjects::table)
            .values(page)
            .execute(&mut store.connection)
            .unwrap();
    }
    seed_request_links(store, project, roots);
}

fn seed_request_links(store: &mut Store, project: &str, roots: &[String]) {
    for start in (0..20_000).step_by(500) {
        let links: Vec<_> = (start..start + 500)
            .map(|i| {
                (
                    work_links::project_id.eq(project),
                    work_links::event_id.eq(format!("member-{i:05}")),
                    work_links::source_scope.eq("activity"),
                    work_links::source_id.eq(format!("source-{i:05}")),
                    work_links::relation.eq("member_of"),
                    work_links::target_scope.eq("session"),
                    work_links::target_id.eq(&roots[i % roots.len()]),
                    work_links::target_json.eq("{}"),
                )
            })
            .collect();
        diesel::insert_into(work_links::table)
            .values(links)
            .execute(&mut store.connection)
            .unwrap();
    }
}
