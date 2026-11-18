// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary test-fixtures: exercise the same JSON contract used by CLI and MCP.
use nexus::audit::AuditAction;
use nexus::persistence::Store;
use nexus::work_history::WorkActor;
use serde_json::{json, Value};

const PROJECT: &str = "audit-integration";
const AT: &str = "2026-11-17T12:00:00Z";

fn actor() -> WorkActor {
    WorkActor {
        agent: "codex".into(),
        session_id: "audit-test".into(),
        model_id: None,
        config_hash: "config".into(),
    }
}

fn append_action(id: &str, event: Value) -> AuditAction {
    serde_json::from_value(json!({
        "action":"append","record":{"id":id,"occurred_at":AT,"event":event}
    }))
    .unwrap()
}

fn append(store: &mut Store, id: &str, event: Value) -> Value {
    let response = store
        .audit_command(PROJECT, &actor(), &append_action(id, event))
        .unwrap();
    serde_json::to_value(response).unwrap()
}

fn read(store: &mut Store, input: Value) -> Value {
    let action: AuditAction = serde_json::from_value(input).unwrap();
    let response = store.audit_command(PROJECT, &actor(), &action).unwrap();
    serde_json::to_value(response).unwrap()
}

fn define(store: &mut Store) {
    let definition = json!({"kind":"define","control":{"key":"review","version":1},
        "definition":{"owner":"Engineering","objective":"Review staged changes","scope":"Nexus",
        "trigger":"Before committing","procedure":"Run both passes","effective_at":"2026-11-01T00:00:00Z",
        "reason":"Initial procedure","results":{"kind":"categorical","codes":[{"code":"pass","label":"Pass"}]},
        "finding_categories":[{"code":"P1","label":"High","order":1}]}});
    let first = append(store, "define-review", definition.clone());
    assert_eq!(first, append(store, "define-review", definition.clone()));
    let mut edited = definition;
    edited["definition"]["owner"] = json!("Another owner");
    let error = store
        .audit_command(PROJECT, &actor(), &append_action("define-review", edited))
        .unwrap_err();
    assert!(error.to_string().contains("different content"));
}

fn initial_result() -> Value {
    json!({"status":"completed","output":{"kind":"code","value":"pass"},
        "findings":[{"id":"finding-1","description":"Stale option","category":"P1","recommendation":"Remove it"}]})
}

fn record_run(store: &mut Store) {
    let run = json!({"kind":"run","control":{"key":"review","version":1},
        "run":{"subject":{"kind":"external","source":"github","id":"pr-12@head"},
        "executor":"review model","result":initial_result()}});
    let mut invalid = run.clone();
    invalid["run"]["result"]["findings"][0]["category"] = json!("unknown");
    let action = append_action("first-run", invalid);
    assert!(store.audit_command(PROJECT, &actor(), &action).is_err());
    append(store, "first-run", run);
}

fn check_followup(store: &mut Store) {
    append(
        store,
        "fixed",
        json!({"kind":"disposition","run_id":"first-run","finding_id":"finding-1",
        "disposition":"remediated","reason":"Removed option","evidence":[{"kind":"external","uri":"https://example.org/fix","description":"Fix commit"}]}),
    );
    let stale = append_action(
        "stale",
        json!({"kind":"disposition","run_id":"first-run",
        "finding_id":"finding-1","disposition":"reopened","reason":"Stale request"}),
    );
    assert!(store
        .audit_command(PROJECT, &actor(), &stale)
        .unwrap_err()
        .to_string()
        .contains("predecessor"));
    append(
        store,
        "verified",
        json!({"kind":"verification","run_id":"first-run","disposition_id":"fixed",
        "verifier":"independent reviewer","procedure":"Inspect the fix","outcome":"confirmed","reason":"Option is gone"}),
    );
    let detail = read(store, json!({"action":"get","run_id":"first-run"}));
    assert_eq!(detail["records"].as_array().unwrap().len(), 3);
    assert_eq!(detail["definition"]["input"]["id"], "define-review");
}

fn check_filters_and_fence(store: &mut Store) {
    let page = read(
        store,
        json!({"action":"list","filter":{"control_key":"review",
        "from":AT,"until":"2026-11-18T00:00:00Z"}}),
    );
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["open_findings"], 0);
    let outside = read(store, json!({"action":"list","filter":{"until":AT}}));
    assert_eq!(outside["total"], 0);
    let frozen = read(
        store,
        json!({"action":"get","run_id":"first-run","cutoff":2}),
    );
    assert_eq!(frozen["records"].as_array().unwrap().len(), 1);
}

fn check_version_and_correction(store: &mut Store) {
    append(
        store,
        "define-review-2",
        json!({"kind":"define","previous":"define-review",
        "control":{"key":"review","version":2},
        "definition":{"owner":"Engineering","objective":"Review staged changes","scope":"Nexus",
        "trigger":"Before committing","procedure":"Describe concerns","effective_at":"2026-11-18T00:00:00Z",
        "reason":"Use narrative results","results":{"kind":"narrative"}}}),
    );
    append(
        store,
        "narrative-run",
        json!({"kind":"run","control":{"key":"review","version":2},
        "run":{"subject":{"kind":"external","source":"github","id":"pr-13@head"},
        "executor":"review model","result":{"status":"completed","output":{"kind":"narrative","value":"Checked sources"}}}}),
    );
    let mut corrected = initial_result();
    corrected["findings"][0]["description"] = json!("Unused option");
    append(
        store,
        "correction",
        json!({"kind":"correct_run","run_id":"first-run","previous":"first-run",
        "reason":"Correct description","result":corrected}),
    );
    let detail = read(store, json!({"action":"get","run_id":"first-run"}));
    assert_eq!(
        detail["records"][0]["input"]["event"]["run"]["result"]["findings"][0]["description"],
        "Stale option"
    );
    assert_eq!(
        detail["records"][3]["input"]["event"]["result"]["findings"][0]["description"],
        "Unused option"
    );
}

fn check_retirement(store: &mut Store) {
    append(
        store,
        "retire-2",
        json!({
            "kind":"retire","control":{"key":"review","version":2},"previous":"define-review-2",
            "reason":"Replaced","effective_at":"2026-11-18T00:00:00Z"
        }),
    );
    append(
        store,
        "define-review-3",
        json!({"kind":"define","previous":"retire-2",
        "control":{"key":"review","version":3},
        "definition":{"owner":"Engineering","objective":"Review staged changes","scope":"Nexus",
        "trigger":"Before committing","procedure":"Use new instructions","effective_at":"2026-11-19T00:00:00Z",
        "reason":"Reactivate","results":{"kind":"narrative"}}}),
    );
    let prior = read(
        store,
        json!({"action":"control","control":{"key":"review","version":2}}),
    );
    assert_eq!(prior["retired"], true);
}
fn exercise(store: &mut Store) {
    define(store);
    record_run(store);
    check_followup(store);
    check_filters_and_fence(store);
    check_version_and_correction(store);
    check_retirement(store);
}

fn compete(open: impl Fn() -> Store + Sync) {
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let update =
            |id| {
                let mut store = open();
                barrier.wait();
                store.audit_command(PROJECT, &actor(), &append_action(id, json!({
                "kind":"disposition","run_id":"first-run","finding_id":"finding-1",
                "previous":"fixed","disposition":"reopened","reason":"Recheck the change"
            })))
            };
        let first = scope.spawn(move || update("concurrent-1"));
        let second = scope.spawn(move || update("concurrent-2"));
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results
        .into_iter()
        .find_map(Result::err)
        .unwrap()
        .to_string()
        .contains("predecessor"));
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_audit_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("audit.db");
    let open = || Store::open(&path).unwrap();
    exercise(&mut open());
    compete(open);
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    open().write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut restored = Store::open(&directory.path().join("restore.db")).unwrap();
    restored.import_streamed_snapshot(snapshot.path()).unwrap();
    assert_eq!(restored.list_projects().unwrap()[0].project_id, PROJECT);
    assert_eq!(
        read(&mut restored, json!({"action":"get","run_id":"first-run"})),
        read(&mut open(), json!({"action":"get","run_id":"first-run"}))
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_audit_lifecycle() {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").unwrap();
    let open = || Store::connect(&url).unwrap();
    exercise(&mut open());
    compete(open);
}
