// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @boundary concern-fixture-wire: decode typed controlled integration inputs.
use nexus::code_concerns::{ConcernAction, ConcernActor};
use nexus::persistence::Store;
use serde_json::{json, Value};
pub fn call(store: &mut Store, action: Value) -> Value {
    let actor = ConcernActor {
        agent: "codex".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let action: ConcernAction = serde_json::from_value(action).unwrap();
    serde_json::to_value(store.concern_command("project", &actor, &action).unwrap()).unwrap()
}
pub fn summary(store: &mut Store, id: &str) -> Value {
    call(store, json!({"action":"get","concern_id":id}))["concern"]["summary"].clone()
}
pub fn fixture() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("concerns.db")).unwrap();
    (directory, store)
}
#[path = "concern_input.rs"]
mod input;
pub use input::*;
