// @feature evals
// @feature persistence
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @boundary typed-fixture-and-query-plan: synthetic records exercise the PostgreSQL prefix access path.
#![cfg(feature = "postgres")]
#[path = "support/eval_record.rs"]
mod eval_record;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use nexus::evals::{EvalProvenance, EvalQuery};
use nexus::persistence::Store;
use serde_json::json;

#[derive(QueryableByName)]
struct PrefixPlan {
    #[diesel(sql_type = diesel::sql_types::Text, column_name = "QUERY PLAN")]
    plan: String,
}

#[test]
fn postgres_eval_id_prefix_is_literal_and_uses_a_bounded_index_range() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").expect("PostgreSQL fixture URL required");
    let mut store = Store::connect(&url).unwrap();
    let project = format!("eval-prefix-{}", uuid::Uuid::new_v4());
    let actor = EvalProvenance {
        agent: "fixture".into(),
        session_id: "prefix-query".into(),
        model_id: None,
        config_hash: "prefix-fixture".into(),
    };
    let records = (0..2000).map(|i| {
        let id = match i {
            0 => "Sample_%!/α".into(), 1 => "Sample_%!/β".into(),
            2 => "sample_%!/wrong-case".into(), _ => format!("unrelated/{i:04}"),
        };
        eval_record::record(&id, json!({"kind":"change","motivation":{"state":"missing","reason":"Synthetic prefix fixture"}}))
    }).collect::<Vec<_>>();
    for batch in records.chunks(64) {
        store.record_eval_bundle(&project, batch, &actor).unwrap();
    }
    let query: EvalQuery =
        serde_json::from_value(json!({"kind":"change","id_prefix":"Sample_%!/"})).unwrap();
    let page = store.list_eval_records(&project, &query).unwrap();
    assert_eq!(
        page.records
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["Sample_%!/α", "Sample_%!/β"]
    );
    assert!(page.next_cursor.is_none());
    assert!(store
        .list_eval_records("other-project", &query)
        .unwrap()
        .records
        .is_empty());
    assert_prefix_plan(&url, &project);
}

fn assert_prefix_plan(url: &str, project: &str) {
    let mut connection = PgConnection::establish(url).unwrap();
    connection.batch_execute("ANALYZE eval_records").unwrap();
    let plan = diesel::sql_query("EXPLAIN SELECT * FROM eval_records WHERE project_id=$1 AND kind='change' AND recorded_at<='2030-01-01' AND id LIKE 'Sample!_!%!!/%' ESCAPE '!' AND substr(id,1,10)='Sample_%!/' ORDER BY id LIMIT 65")
        .bind::<diesel::sql_types::Text,_>(project).load::<PrefixPlan>(&mut connection).unwrap();
    let text = plan
        .into_iter()
        .map(|r| r.plan)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("eval_records_id_prefix") && text.contains("~>=~") && text.contains("~<~"),
        "prefix must use its index bounds: {text}"
    );
}
