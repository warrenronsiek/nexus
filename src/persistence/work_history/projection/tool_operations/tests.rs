// @feature persistence
// @feature work-history
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
// @boundary disposable-postgres-plan: EXPLAIN inspects the captured production query with test-owned literal bindings in NEXUS_TEST_DATABASE_URL only.
use super::*;
use crate::persistence::{test_support::capture_queries, Store};
use diesel::connection::SimpleConnection;

#[derive(QueryableByName)]
struct Plan {
    #[diesel(sql_type = diesel::sql_types::Text)]
    #[diesel(column_name = "QUERY PLAN")]
    line: String,
}

#[test]
fn session_operation_lookup_does_not_scan_unrelated_history() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").expect("disposable PostgreSQL URL required");
    let mut store = Store::connect(&url).unwrap();
    store.connection.begin_test_transaction().unwrap();
    let project = format!("operation-scale-{}", uuid::Uuid::new_v4());
    let ids: Vec<_> = (0..20_000).map(|i| format!("{project}-{i:05}")).collect();
    seed_history(&mut store.connection, &project, &ids);
    store
        .connection
        .batch_execute("ANALYZE analytics_calls; ANALYZE analytics_actions;")
        .unwrap();
    let queries = capture_queries(&mut store.connection);
    let rows = operation_rows(&mut store.connection, &project, &ids[300]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].invocation, ids[300]);
    let sql = queries.lock().unwrap()[0]
        .split(" -- binds:")
        .next()
        .unwrap()
        .to_owned()
        .replace("$4", "8193")
        .replace("$3", &format!("'{}'", ids[300]))
        .replace("$2", &format!("'{project}'"))
        .replace("$1", &format!("'{project}'"));
    let plan = diesel::sql_query(format!("EXPLAIN {sql}"))
        .load::<Plan>(&mut store.connection)
        .unwrap();
    let plan = plan
        .into_iter()
        .map(|row| row.line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !plan.contains("Seq Scan on analytics_calls")
            && !plan.contains("Seq Scan on analytics_actions"),
        "one session lookup must not scan unrelated calls/actions:\n{plan}"
    );
}

fn seed_history(connection: &mut DbConnection, project: &str, ids: &[String]) {
    seed_category(connection, project);
    for page in ids.chunks(500) {
        let call_rows: Vec<_> = page
            .iter()
            .map(|id| {
                (
                    calls::id.eq(id),
                    calls::project_id.eq(project),
                    calls::agent.eq("test"),
                    calls::session_key.eq(id),
                    calls::invocation_id.eq(id),
                    calls::tool_name.eq("Read"),
                    calls::outcome.eq("succeeded"),
                    calls::observed_at.eq("2026-01-01T00:00:00Z"),
                )
            })
            .collect();
        diesel::insert_into(calls::table)
            .values(&call_rows)
            .execute(connection)
            .unwrap();
        let action_rows: Vec<_> = page
            .iter()
            .map(|id| {
                (
                    actions::id.eq(id),
                    actions::call_id.eq(id),
                    actions::project_id.eq(project),
                    actions::session_key.eq(id),
                    actions::resource_key.eq("fixture"),
                    actions::name.eq("Read source"),
                    actions::kind.eq("tool"),
                    actions::origin.eq("host_tool"),
                    actions::category_id.eq(project),
                    actions::description.eq("Fixture operation"),
                    actions::pattern_key.eq("fixture"),
                    actions::evidence.eq("deterministic"),
                    actions::observed_at.eq("2026-01-01T00:00:00Z"),
                )
            })
            .collect();
        diesel::insert_into(actions::table)
            .values(&action_rows)
            .execute(connection)
            .unwrap();
    }
}

fn seed_category(connection: &mut DbConnection, project: &str) {
    diesel::insert_into(categories::table)
        .values((
            categories::id.eq(project),
            categories::name.eq("Test"),
            categories::definition.eq("Fixture"),
            categories::created_at.eq("2026-01-01"),
            categories::updated_at.eq("2026-01-01"),
        ))
        .execute(connection)
        .unwrap();
}
