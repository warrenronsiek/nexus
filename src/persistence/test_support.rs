// @feature persistence
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary disposable-postgres-plan: EXPLAIN reads controlled captured-query SQL from tests, never caller-supplied application SQL.
use super::backend::DbConnection;
use crate::papercuts::PapercutActor;
use crate::sos::SosActor;
use diesel::Connection;
use std::sync::{Arc, Mutex};

pub(super) trait TestActor: Sized {
    fn for_session(session_id: &str) -> Self;
}

macro_rules! impl_test_actor {
    ($actor:ident, $model:literal, $config_hash:literal) => {
        impl TestActor for $actor {
            fn for_session(session_id: &str) -> Self {
                Self {
                    agent: "codex".to_owned(),
                    session_id: session_id.to_owned(),
                    model_id: Some($model.to_owned()),
                    config_hash: $config_hash.to_owned(),
                }
            }
        }
    };
}

impl_test_actor!(PapercutActor, "test-model", "test-config");
impl_test_actor!(SosActor, "gpt-test", "config-a");

pub(super) fn actor<Actor: TestActor>(session_id: &str) -> Actor {
    Actor::for_session(session_id)
}

pub(super) type QueryLog = Arc<Mutex<Vec<String>>>;

pub(super) fn capture_queries(connection: &mut DbConnection) -> QueryLog {
    let queries = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&queries);
    connection.set_instrumentation(move |event: diesel::connection::InstrumentationEvent<'_>| {
        if let diesel::connection::InstrumentationEvent::StartQuery { query, .. } = event {
            captured.lock().unwrap().push(query.to_string());
        }
    });
    queries
}

/// Diagnostics accept only controlled SELECTs and the limited bind shapes used by tests.
#[cfg(feature = "postgres")]
pub(super) enum QueryPlanMode {
    Estimated,
    Analyzed,
}

#[cfg(feature = "postgres")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum PlanBind {
    Text(String),
    Integer(i64),
    Texts(Vec<String>),
}

#[cfg(feature = "postgres")]
impl PlanBind {
    fn literal(&self) -> String {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
        match self {
            Self::Text(value) => quote(value),
            Self::Integer(value) => value.to_string(),
            Self::Texts(values) => format!(
                "ARRAY[{}]::text[]",
                values
                    .iter()
                    .map(|s| quote(s))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

#[cfg(feature = "postgres")]
pub(super) fn postgres_query_plan(
    connection: &mut DbConnection,
    captured: &str,
    mode: QueryPlanMode,
) -> String {
    use diesel::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Line {
        #[diesel(sql_type = diesel::sql_types::Text)]
        #[diesel(column_name = "QUERY PLAN")]
        text: String,
    }
    let (query, values) = captured
        .rsplit_once(" -- binds: ")
        .expect("captured typed query bindings");
    assert!(
        query.starts_with("SELECT"),
        "plan diagnostics are read-only"
    );
    let values: Vec<PlanBind> =
        serde_json::from_str(values).expect("controlled text/integer/array test bindings");
    let query = values
        .iter()
        .enumerate()
        .rev()
        .fold(query.to_owned(), |sql, (index, value)| {
            sql.replace(&format!("${}", index + 1), &value.literal())
        });
    let explain = match mode {
        QueryPlanMode::Estimated => "EXPLAIN",
        QueryPlanMode::Analyzed => "EXPLAIN (ANALYZE, COSTS OFF, SUMMARY OFF)",
    };
    diesel::sql_query(format!("{explain} {query}"))
        .load::<Line>(connection)
        .unwrap()
        .into_iter()
        .map(|row| row.text)
        .collect::<Vec<_>>()
        .join("\n")
}
