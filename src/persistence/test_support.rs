// @feature persistence
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
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
