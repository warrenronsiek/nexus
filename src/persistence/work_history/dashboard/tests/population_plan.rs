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
