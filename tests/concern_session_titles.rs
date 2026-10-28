// @feature code-concerns
// @feature work-history
// @spec docs/features/code-concerns.md
// @spec docs/features/work-history.md
// @boundary source-session-title-fixture: exact host IDs and work observations provide the displayed names
#![cfg(feature = "sqlite")]
use nexus::{
    code_concerns::{ConcernAction, ConcernActor},
    persistence::Store,
    work_history::{WorkAction, WorkActor},
};
use serde_json::{json, Value};

#[test]
fn concern_detail_resolves_exact_session_titles_without_changing_observations() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("titles.db")).unwrap();
    seed_titles(&mut store);
    let actor = ConcernActor {
        agent: "codex".into(),
        session_id: "reporter".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let observation = json!({"id":"report","concern_id":"issue","observed_at":"2026-10-27T12:01:00Z",
        "context":{"host_session_id":"host-one","work_session_id":"delivery"},
        "body":{"event":"reported","title":"Fallback drops state","description":"State is absent in fallback","encounter":{}}});
    let action: ConcernAction =
        serde_json::from_value(json!({"action":"record","observations":[observation.clone()]}))
            .unwrap();
    store.concern_command("project", &actor, &action).unwrap();
    let get: ConcernAction =
        serde_json::from_value(json!({"action":"get","concern_id":"issue"})).unwrap();
    let detail =
        serde_json::to_value(store.concern_command("project", &actor, &get).unwrap()).unwrap();
    assert_eq!(
        detail["concern"]["session_titles"][0]["host_title"],
        "Investigate dropped context"
    );
    assert_eq!(
        detail["concern"]["session_titles"][0]["work_title"],
        "Fix router fallback"
    );
    assert_eq!(
        detail["concern"]["history"][0]["input"],
        observation_with_defaults(observation)
    );
    let history: ConcernAction =
        serde_json::from_value(json!({"action":"history","concern_id":"issue"})).unwrap();
    let history =
        serde_json::to_value(store.concern_command("project", &actor, &history).unwrap()).unwrap();
    assert_eq!(
        history["session_titles"],
        detail["concern"]["session_titles"]
    );
    assert_imported_titles(&mut store, &detail);
}

fn observation_with_defaults(value: Value) -> Value {
    serde_json::to_value(
        serde_json::from_value::<nexus::code_concerns::ConcernObservationInput>(value).unwrap(),
    )
    .unwrap()
}

fn seed_titles(store: &mut Store) {
    let mut hasher = blake3::Hasher::new();
    for part in ["agent", "codex", "host-one"] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let activity = hasher.finalize().to_hex().to_string();
    let work_event = |id: &str, subject: Value, body: Value| {
        json!({"id":id,"subject":subject,"occurred_at":"2026-10-27T12:00:00Z",
        "producer":{"name":"test","version":"1","instance_id":"source"},"method":"host_hook_boundary","body":body})
    };
    let work:WorkAction=serde_json::from_value(json!({"action":"record","observations":[
        work_event("open",json!({"scope":"session","id":"delivery"}),json!({"event":"session_opened","title":"Fix router fallback"})),
        work_event("host",json!({"scope":"activity","id":activity}),json!({"event":"activity_declared","name":"Host session","kind":"host.session","work_session_id":"delivery","context":{"host_session_id":"host-one","labels":{"nexus.capture.host":"codex"}}})),
        work_event("title",json!({"scope":"activity","id":activity}),json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":"Investigate dropped context"}}}))
    ]})).unwrap();
    store
        .work_command(
            "project",
            &WorkActor {
                agent: "codex".into(),
                session_id: "host-one".into(),
                model_id: None,
                config_hash: "test".into(),
            },
            &work,
        )
        .unwrap();
}
fn assert_imported_titles(store: &mut Store, detail: &Value) {
    let mut imported = detail["concern"]["history"][0].clone();
    imported["input"]["id"] = "imported-report".into();
    imported["input"]["concern_id"] = "imported-concern".into();
    imported["input"]["body"]["title"] = "Imported fallback report".into();
    let import: ConcernAction =
        serde_json::from_value(json!({"action":"import","observations":[imported]})).unwrap();
    let importer = ConcernActor {
        agent: "claude".into(),
        session_id: "importer".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    store
        .concern_command("project", &importer, &import)
        .unwrap();
    let get: ConcernAction =
        serde_json::from_value(json!({"action":"get","concern_id":"imported-concern"})).unwrap();
    let detail =
        serde_json::to_value(store.concern_command("project", &importer, &get).unwrap()).unwrap();
    assert_eq!(detail["concern"]["session_titles"][0]["agent"], "codex");
    assert_eq!(
        detail["concern"]["session_titles"][0]["host_title"],
        "Investigate dropped context"
    );
}
