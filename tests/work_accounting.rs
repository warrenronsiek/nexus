// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: explicit source boundaries are decoded by the typed work API.
#![cfg(feature = "sqlite")]
use nexus::persistence::Store;
use serde_json::json;
#[path = "support/work_accounting.rs"]
mod fixture;
use fixture::{at, command, interval};

fn record_parallel_session(store: &mut Store) {
    let mut observations = vec![
        at(
            "root-open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root","context":{}}),
        ),
        at(
            "root-finish",
            "session",
            "root",
            100,
            json!({"event":"finished","outcome":"unknown"}),
        ),
    ];
    observations.extend(interval("parent", None, Some("worker-a"), 0, 90));
    observations.extend(interval("test-a", Some("parent"), Some("worker-a"), 10, 30));
    observations.extend(interval("test-b", Some("parent"), Some("worker-b"), 20, 50));
    observations.extend(interval("wait", Some("parent"), None, 70, 80));
    for observation in &mut observations {
        if observation["body"]["event"] == "activity_declared" {
            let role = match observation["subject"]["id"].as_str().unwrap() {
                "parent" => "container",
                "wait" => "wait",
                _ => "execution",
            };
            observation["body"]["context"]["labels"] = json!({"nexus.accounting.role":role});
        }
    }
    command(
        store,
        json!({"action":"record","observations":observations}),
    );
}
#[test]
fn session_accounting_distinguishes_overlap_inclusive_worker_time_and_unobserved_gaps() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("accounting.db")).unwrap();
    record_parallel_session(&mut store);
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    let accounting = &result["accounting"];
    assert_eq!(accounting["session_wall"]["nanoseconds"], "100000000000");
    assert_eq!(accounting["occupied_wall"]["seconds"], 50.0);
    assert_eq!(
        accounting["inclusive_execution"]["nanoseconds"],
        "150000000000"
    );
    assert_eq!(accounting["worker_occupancy"]["seconds"], 120.0);
    assert_eq!(accounting["unobserved"]["seconds"], 50.0);
    assert_eq!(
        accounting["unobserved_intervals"][0]["started_at"],
        "2026-10-27T12:00:00Z"
    );
    assert_eq!(accounting["coverage"]["activity_count"], 4);
    assert_eq!(accounting["coverage"]["unknown_executor_count"], 1);
    assert_eq!(
        accounting["coverage"]["capability_inventory"],
        "unavailable"
    );
    assert_eq!(result["intervals"][0]["executor"], "worker-a");
    assert!(accounting["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("not CPU")));
}
#[test]
fn missing_boundaries_and_cross_clock_history_remain_unknown_and_reduce_coverage() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("unknown.db")).unwrap();
    let mut observations = vec![
        at(
            "open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root"}),
        ),
        at(
            "end",
            "session",
            "root",
            20,
            json!({"event":"finished","outcome":"unknown"}),
        ),
    ];
    let mut cross = interval("cross", None, None, 2, 10);
    cross[2]["body"].as_object_mut().unwrap().remove("elapsed");
    cross[2]["producer"]["instance_id"] = json!("other-process");
    observations.extend(cross);
    let mut missing = interval("missing", None, None, 3, 12);
    missing.pop();
    observations.extend(missing);
    let mut finish_only = interval("finish-only", None, None, 3, 12);
    finish_only.remove(1);
    observations.extend(finish_only);
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    let coverage = &result["accounting"]["coverage"];
    assert_eq!(coverage["missing_start_count"], 1);
    assert_eq!(coverage["missing_finish_count"], 1);
    assert_eq!(coverage["clock_conflict_count"], 1);
    assert_eq!(coverage["interval_count"], 0);
    assert_eq!(result["accounting"]["occupied_wall"]["seconds"], 0.0);
    assert_eq!(result["accounting"]["unobserved"]["seconds"], 20.0);
    assert!(result["accounting"]["worker_occupancy"].is_null());
}

#[test]
fn unclassified_container_lifetimes_never_claim_attributed_work() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("roles.db")).unwrap();
    let mut observations = vec![
        at(
            "open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root"}),
        ),
        at(
            "end",
            "session",
            "root",
            20,
            json!({"event":"finished","outcome":"unknown"}),
        ),
    ];
    observations.extend(interval("opaque", None, None, 0, 20));
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert_eq!(result["accounting"]["occupied_wall"]["seconds"], 0.0);
    assert_eq!(result["accounting"]["unobserved"]["seconds"], 20.0);
    assert_eq!(result["accounting"]["coverage"]["unknown_role_count"], 1);
    assert_eq!(result["intervals"][0]["accounting_role"], "unknown");
}
#[test]
fn live_session_titles_follow_source_order_and_keep_stable_root_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("titles.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":[
            at("root","session","root",0,json!({"event":"session_opened","title":"Host session"})),
            at("a-later","session","root",20,json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":"Renamed session"}}})),
            at("z-earlier","session","root",10,json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":"Initial title"}}}))
        ]}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert_eq!(result["selected_session"]["title"], "Renamed session");
    assert_eq!(result["selected_session"]["subject"]["id"], "root");
}
#[test]
fn compact_waterfall_keeps_more_than_the_secondary_summary_limit_and_exact_totals() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("many.db")).unwrap();
    command(
        &mut store,
        json!({"action":"record","observations":[at("open","session","root",0,json!({"event":"session_opened","title":"Root"}))]}),
    );
    for batch in 0..11 {
        let mut observations = Vec::new();
        for offset in 0..48 {
            let id = format!("node-{:04}", batch * 48 + offset);
            observations.push(at(&id,"activity",&id,0,json!({"event":"activity_declared","name":"Task","kind":"test","work_session_id":"root"})));
        }
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
    }
    command(
        &mut store,
        json!({"action":"record","observations":[at("precise","activity","node-0000",10,json!({"event":"finished","outcome":"unknown","elapsed":{"nanoseconds":"90071992547409931234567890123456789012345678901234567890123456789012345678901234","method":"source"}}))]}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert_eq!(result["intervals"].as_array().unwrap().len(), 528);
    assert_eq!(result["accounting"]["coverage"]["display_partial"], false);
    assert_eq!(result["accounting"]["coverage"]["activity_count"], 528);
    assert_eq!(
        result["accounting"]["inclusive_execution"]["nanoseconds"],
        "90071992547409931234567890123456789012345678901234567890123456789012345678901234"
    );
    assert!(serde_json::to_vec(&result).unwrap().len() < 1024 * 1024);
}

#[test]
fn adapter_coverage_gaps_and_descendants_outside_the_root_remain_visibly_partial() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("partial-source.db")).unwrap();
    let mut observations = vec![
        at(
            "open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root"}),
        ),
        at(
            "end",
            "session",
            "root",
            10,
            json!({"event":"finished","outcome":"unknown"}),
        ),
        at(
            "resume",
            "session",
            "root",
            12,
            json!({"event":"milestone","name":"Host lifecycle coverage incomplete","context":{"labels":{"nexus.capture.gap":"host_resume_epoch_unavailable"}}}),
        ),
    ];
    let mut child = interval("background", None, None, 2, 20);
    child[0]["body"]["context"] = json!({"labels":{"nexus.accounting.role":"execution"}});
    observations.extend(child);
    observations.push(at("evicted","session","root",13,json!({"event":"milestone","name":"Host lifecycle coverage incomplete","context":{"labels":{"nexus.capture.gap":"request_context_evicted"}}})));
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert!(result["selected_session"]["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("host_resume_epoch_unavailable")));
    assert!(result["selected_session"]["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("request_context_evicted")));
    assert_eq!(result["accounting"]["coverage"]["outside_session_count"], 1);
    assert_eq!(result["accounting"]["coverage"]["source_partial"], true);
    assert_eq!(result["accounting"]["occupied_wall"]["seconds"], 8.0);
    assert_eq!(
        result["intervals"][0]["finished_at"],
        "2026-10-27T12:00:20Z"
    );
}

fn record_legacy_executor(store: &mut Store) {
    let mut observations = vec![
        at(
            "open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root"}),
        ),
        at(
            "end",
            "session",
            "root",
            20,
            json!({"event":"finished","outcome":"unknown"}),
        ),
    ];
    observations.extend(interval("legacy", None, Some("github-actions"), 2, 10));
    observations.extend([
        at("role","activity","legacy",11,json!({"event":"milestone","name":"Accounting role observed","context":{"labels":{"nexus.accounting.role":"execution"}}})),
        at("unknown-executor","activity","legacy",11,json!({"event":"milestone","name":"Executor identity observed","context":{"labels":{"nexus.executor.declaration":"unqualified"}}}))
    ]);
    command(
        store,
        json!({"action":"record","observations":observations}),
    );
}
#[test]
fn additive_accounting_metadata_qualifies_legacy_executor_labels_without_redeclaration() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("executor-metadata.db")).unwrap();
    record_legacy_executor(&mut store);
    let first = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert!(first["intervals"][0]["executor"].is_null());
    assert_eq!(first["intervals"][0]["accounting_role"], "execution");
    assert_eq!(first["accounting"]["occupied_wall"]["seconds"], 8.0);
    assert!(first["accounting"]["worker_occupancy"].is_null());
    command(
        &mut store,
        json!({"action":"record","observations":[at("assigned","activity","legacy",12,json!({"event":"milestone","name":"Executor identity observed","context":{"labels":{"nexus.executor.declaration":"unqualified","nexus.executor.id":"github:example/repository:runner:8123"}}}))]}),
    );
    let second = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    assert_eq!(
        second["intervals"][0]["executor"],
        "github:example/repository:runner:8123"
    );
    assert_eq!(second["accounting"]["worker_occupancy"]["seconds"], 8.0);
    command(
        &mut store,
        json!({"action":"record","observations":[at("unknown-later","activity","legacy",13,json!({"event":"milestone","name":"Executor identity observed","context":{"labels":{"nexus.executor.declaration":"unqualified"}}}))]}),
    );
    let preserved = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"legacy"}}),
    );
    assert_eq!(
        preserved["record"]["summary"]["executor"],
        "github:example/repository:runner:8123"
    );
    command(
        &mut store,
        json!({"action":"record","observations":[at("competing","activity","legacy",14,json!({"event":"milestone","name":"Executor identity observed","context":{"labels":{"nexus.executor.declaration":"unqualified","nexus.executor.id":"github:example/repository:runner:9999"}}}))]}),
    );
    let conflicted = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"legacy"}}),
    );
    assert!(conflicted["record"]["summary"]["executor"].is_null());
    assert!(conflicted["record"]["summary"]["conflicts"]
        .as_array()
        .unwrap()
        .contains(&json!("conflicting_executors")));
}
